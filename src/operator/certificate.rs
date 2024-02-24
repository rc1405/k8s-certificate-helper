use k8s_openapi::api::certificates::v1::{
    CertificateSigningRequest, CertificateSigningRequestCondition, CertificateSigningRequestSpec,
    CertificateSigningRequestStatus,
};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::ByteString;
use kube::core::ResourceExt;
use kube::Client;
use kube::{core::ObjectMeta, Api};
use rcgen::{date_time_ymd, Certificate, CertificateParams, DistinguishedName, DnType, SanType};
use std::collections::BTreeMap;
use tracing::info;

use crate::controller::Error;
use crate::crd::{Certificate as CertificateHelper, Stage};

use super::{perform_cluster_operation, perform_operation};
use super::{perform_get, update_status, Operation};

pub struct CertificateStage {
    client: Client,
    operation: Operation,
    certificate: CertificateHelper,
    cert: Option<Certificate>,
    csr_request: Option<CertificateSigningRequest>,
    signed_cert: Option<ByteString>,
    cert_creation_time: Option<Time>,
    secret: Option<Secret>,
}

impl CertificateStage {
    pub fn new(
        client: Client,
        operation: Operation,
        certificate: CertificateHelper,
    ) -> CertificateStage {
        CertificateStage {
            client,
            operation,
            certificate,
            cert: None,
            csr_request: None,
            signed_cert: None,
            cert_creation_time: None,
            secret: None,
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        match self.operation {
            Operation::Create => {
                self.generate_cert().await?;
                self.create_csr().await?;
                self.approve_csr().await?;
                self.create_secret().await?;
                self.delete_csr().await?;
            }
            Operation::Delete => {
                if let Some(status) = self.certificate.status.clone() {
                    if let Some(secret) = status.certificate {
                        let secret: Secret = perform_get(
                            self.client.clone(),
                            &secret,
                            &self.certificate.spec.namespace,
                        )
                        .await?;
                        self.secret = Some(secret);
                    };
                };
                self.delete().await?;
                return Ok(());
            }
            _ => {}
        };

        let csr_name = match self.csr_request.clone() {
            Some(csr) => csr.name_any(),
            None => "<unknown>".into(),
        };
        update_status(
            self.client.clone(),
            Stage::CertificateCreated(csr_name),
            self.certificate.clone(),
        )
        .await?;

        if let Some(uid) = self.certificate.uid() {
            if let Some(secret) = self.secret.clone() {
                perform_operation(self.client.clone(), Operation::ApplyOwner(uid), &secret).await?;
            };
        };
        Ok(())
    }

    async fn generate_cert(&mut self) -> Result<(), Error> {
        let mut params: CertificateParams = Default::default();
        params.not_before = date_time_ymd(1975, 1, 1);
        params.not_after = date_time_ymd(4096, 1, 1);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::OrganizationName, "system:nodes");
        params.distinguished_name.push(
            DnType::CommonName,
            format!(
                "system:node:{}",
                self.certificate.spec.service.to_lowercase()
            ),
        );

        let mut alt_names = vec![SanType::DnsName(
            self.certificate.spec.service.to_lowercase(),
        )];
        for i in self.certificate.spec.alt_names.clone().unwrap_or_default() {
            alt_names.push(SanType::DnsName(i));
        }

        params.subject_alt_names = alt_names;
        self.cert = Some(Certificate::from_params(params)?);
        Ok(())
    }

    async fn create_csr(&mut self) -> Result<(), Error> {
        let raw_csr = self.cert.as_ref().unwrap().serialize_request_pem()?;
        let request = CertificateSigningRequest {
            metadata: ObjectMeta {
                name: Some(self.certificate.name_any().to_lowercase()),
                ..Default::default()
            },
            spec: CertificateSigningRequestSpec {
                expiration_seconds: Some(86400),
                signer_name: "kubernetes.io/kubelet-serving".into(),
                request: ByteString(raw_csr.into_bytes().to_vec()),
                usages: Some(vec![
                    "key encipherment".into(),
                    "digital signature".into(),
                    "server auth".into(),
                ]),
                ..Default::default()
            },
            ..Default::default()
        };
        let _response =
            perform_cluster_operation(self.client.clone(), Operation::Create, &request).await?;
        self.csr_request = Some(request);
        info!(
            "Certificate {} created",
            self.certificate.name_any().to_lowercase()
        );
        Ok(())
    }

    async fn approve_csr(&mut self) -> Result<(), Error> {
        let csr_api: Api<CertificateSigningRequest> = Api::all(self.client.clone());
        let mut request = self.csr_request.clone().unwrap();

        request.status = Some(CertificateSigningRequestStatus {
            certificate: None,
            conditions: Some(vec![CertificateSigningRequestCondition {
                type_: String::from("Approved"),
                message: Some(String::from("Approved by webhook-helper")),
                reason: Some(String::from("WebHelperApproved")),
                status: String::from("True"),
                last_transition_time: None,
                last_update_time: self.cert_creation_time.clone(),
            }]),
        });

        let body: Vec<u8> = serde_json::to_vec(&request)?;
        let url = format!(
            "/apis/certificates.k8s.io/v1/certificatesigningrequests/{}/approval",
            self.certificate.name_any().to_lowercase()
        );
        let req = http::request::Request::put(url).body(body)?;

        // Deserialize JSON response as a JSON value. Alternatively, a type that
        // implements `Deserialize` can be used.
        let resp = self
            .client
            .request::<CertificateSigningRequest>(req)
            .await?;

        // waiter for status ok
        let mut cert_with_approval = csr_api.get_approval(resp.name_any().as_str()).await?;
        loop {
            if let Some(status) = cert_with_approval.status.clone() {
                if let Some(certificate) = status.certificate.clone() {
                    self.signed_cert = Some(certificate);
                    break;
                };
            };

            std::thread::sleep(std::time::Duration::from_secs(5));
            cert_with_approval = csr_api.get_approval(resp.name_any().as_str()).await?;
        }

        info!(
            "Certificate {} approved",
            self.certificate.name_any().to_lowercase()
        );
        Ok(())
    }

    async fn create_secret(&mut self) -> Result<(), Error> {
        let key = self
            .cert
            .as_ref()
            .unwrap()
            .serialize_private_key_pem()
            .as_bytes()
            .to_vec();
        let cert = ByteString(self.signed_cert.as_ref().unwrap().0.clone());

        let mut data: BTreeMap<String, ByteString> = BTreeMap::new();
        data.insert("tls.key".into(), ByteString(key));
        data.insert("tls.crt".into(), cert);

        let secret = Secret {
            type_: Some(format!(
                "{}/tls",
                self.certificate.name_any().to_lowercase()
            )),
            metadata: ObjectMeta {
                name: Some(self.certificate.name_any().to_lowercase().to_string()),
                namespace: Some(self.certificate.spec.namespace.clone()),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        let result = perform_operation(self.client.clone(), Operation::Create, &secret).await?;
        self.secret = Some(result);

        info!(
            "Secret {} created",
            self.certificate.name_any().to_lowercase()
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_secret(&self) -> Option<Secret> {
        self.secret.clone()
    }

    async fn delete(&self) -> Result<(), Error> {
        if let Some(secret) = self.secret.clone() {
            perform_operation(self.client.clone(), Operation::Delete, &secret).await?;
        };
        Ok(())
    }

    async fn delete_csr(&mut self) -> Result<(), Error> {
        if let Some(csr_request) = self.csr_request.clone() {
            let _response =
                perform_cluster_operation(self.client.clone(), Operation::Delete, &csr_request)
                    .await?;
        };
        info!("CSR {} deleted", self.certificate.name_any().to_lowercase());
        Ok(())
    }
}
