use k8s_openapi::api::admissionregistration::v1::ServiceReference;
use k8s_openapi::api::admissionregistration::v1::{
    MutatingWebhookConfiguration, ValidatingWebhookConfiguration,
};
use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::ByteString;
use kube::core::ResourceExt;
use kube::Client;

use serde_json::Value;

use super::{
    determine_stage, perform_cluster_get, perform_cluster_operation, perform_get, update_status,
    Operation,
};
use crate::controller::Error;
use crate::crd::{Stage, WebhookHelper, WebhookType};

pub struct WebhookStage {
    client: Client,
    operation: Operation,
    webhook: WebhookHelper,
    service: Option<Service>,
    webhook_resource: Option<WebhookType>,
}

impl WebhookStage {
    pub fn new(
        client: Client,
        operation: Operation,
        webhook: WebhookHelper,
        service: Option<Service>,
    ) -> WebhookStage {
        WebhookStage {
            client,
            operation,
            webhook,
            service,
            webhook_resource: None,
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        match self.operation {
            Operation::Bootstrap => {
                if self.service.is_none() {
                    return Err(Error::UnknownOperation("Service is not known".into()));
                };
                self.create_webhook().await?;
                return Ok(());
            }
            Operation::Delete => {
                if let Some(status) = self.webhook.status.clone() {
                    if let Some(webhook) = status.validating_webhook {
                        let hook: ValidatingWebhookConfiguration =
                            perform_cluster_get(self.client.clone(), &webhook).await?;
                        self.webhook_resource = Some(WebhookType::Validating(hook));
                    } else if let Some(webhook) = status.mutating_webhook {
                        let hook: MutatingWebhookConfiguration =
                            perform_cluster_get(self.client.clone(), &webhook).await?;
                        self.webhook_resource = Some(WebhookType::Mutating(hook));
                    } else {
                        return Ok(());
                    };
                    self.delete().await?;
                    return Ok(());
                }
            }
            _ => {
                let stage = determine_stage(self.client.clone(), self.webhook.clone()).await?;
                match stage {
                    Stage::ServiceCreated(service) => {
                        self.service = Some(service);
                        let webhook = self.create_webhook().await?;
                        update_status(
                            self.client.clone(),
                            Stage::WebhookCreated(webhook.clone()),
                            self.webhook.clone(),
                        )
                        .await?;

                        if let Some(uid) = self.webhook.uid() {
                            match webhook {
                                WebhookType::Mutating(d) => {
                                    perform_cluster_operation(
                                        self.client.clone(),
                                        Operation::ApplyOwner(uid),
                                        &d,
                                    )
                                    .await?;
                                }
                                WebhookType::Validating(p) => {
                                    perform_cluster_operation(
                                        self.client.clone(),
                                        Operation::ApplyOwner(uid),
                                        &p,
                                    )
                                    .await?;
                                }
                            };
                        };
                    }
                    _ => {
                        // check if operation is update
                    }
                };
            }
        };
        Ok(())
    }

    async fn create_webhook(&mut self) -> Result<WebhookType, Error> {
        let config_map: ConfigMap =
            perform_get(self.client.clone(), "kube-root-ca.crt", "default").await?;
        let cluster_ca_string = match config_map.data {
            Some(cm) => match cm.get("ca.crt") {
                Some(c) => c.clone(),
                None => {
                    return Err(Error::UnableToCreateObject(
                        "Cound not find signing CA".into(),
                    ))
                }
            },
            None => {
                return Err(Error::UnableToCreateObject(
                    "Cound not find signing CA".into(),
                ))
            }
        };

        if let Some(service) = self.service.clone() {
            if let Ok(mut hook) =
                convert_to_mutating_webhook(self.webhook.spec.webhook.clone()).await
            {
                let current_webhooks = hook
                    .webhooks
                    .unwrap_or(Vec::new())
                    .iter()
                    .map(|w| {
                        let mut nwh = w.clone();
                        nwh.client_config.url = None;
                        nwh.client_config.ca_bundle =
                            Some(ByteString(cluster_ca_string.as_bytes().into()));
                        nwh.client_config.service = Some(ServiceReference {
                            name: service.name_any(),
                            namespace: self.webhook.namespace().unwrap_or("default".into()),
                            port: Some(self.webhook.spec.listening_port),
                            path: None,
                        });
                        nwh
                    })
                    .collect();
                hook.webhooks = Some(current_webhooks);

                let result =
                    perform_cluster_operation(self.client.clone(), Operation::Create, &hook)
                        .await?;
                self.webhook_resource = Some(WebhookType::Mutating(result.clone()));
                Ok(WebhookType::Mutating(result))
            } else if let Ok(mut hook) =
                convert_to_admission_webhook(self.webhook.spec.webhook.clone()).await
            {
                let current_webhooks = hook
                    .webhooks
                    .unwrap_or(Vec::new())
                    .iter()
                    .map(|w| {
                        let mut nwh = w.clone();
                        nwh.client_config.url = None;
                        nwh.client_config.ca_bundle =
                            Some(ByteString(cluster_ca_string.as_bytes().into()));
                        nwh.client_config.service = Some(ServiceReference {
                            name: service.name_any(),
                            namespace: service.namespace().unwrap_or("default".into()),
                            port: Some(self.webhook.spec.listening_port),
                            path: self.webhook.spec.path.clone(),
                        });
                        nwh
                    })
                    .collect();
                hook.webhooks = Some(current_webhooks);

                let result =
                    perform_cluster_operation(self.client.clone(), Operation::Create, &hook)
                        .await?;
                self.webhook_resource = Some(WebhookType::Validating(result.clone()));
                Ok(WebhookType::Validating(result))
            } else {
                Err(Error::UnknownOperation(
                    "Unable to determine webhook type".into(),
                ))
            }
        } else {
            Err(Error::UnknownOperation(
                "Unable to determine webhook type".into(),
            ))
        }
    }

    #[allow(dead_code)]
    pub async fn get_webhook(&self) -> Option<WebhookType> {
        self.webhook_resource.clone()
    }

    pub async fn delete(&self) -> Result<(), Error> {
        if let Some(webhook) = self.webhook_resource.clone() {
            match webhook {
                WebhookType::Mutating(m) => {
                    perform_cluster_operation(self.client.clone(), Operation::Delete, &m).await?;
                }
                WebhookType::Validating(v) => {
                    perform_cluster_operation(self.client.clone(), Operation::Delete, &v).await?;
                }
            }
        };
        Ok(())
    }
}

async fn convert_to_mutating_webhook(data: Value) -> Result<MutatingWebhookConfiguration, Error> {
    let value: MutatingWebhookConfiguration = serde_json::from_value(data)?;
    Ok(value)
}

async fn convert_to_admission_webhook(
    data: Value,
) -> Result<ValidatingWebhookConfiguration, Error> {
    let value: ValidatingWebhookConfiguration = serde_json::from_value(data)?;
    Ok(value)
}
