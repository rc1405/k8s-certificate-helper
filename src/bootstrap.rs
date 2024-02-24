use std::collections::BTreeMap;

use k8s_openapi::api::admissionregistration::v1::RuleWithOperations;
use k8s_openapi::api::admissionregistration::v1::WebhookClientConfig;
use k8s_openapi::api::admissionregistration::v1::{
    ValidatingWebhook, ValidatingWebhookConfiguration,
};
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::core::ObjectMeta;
use kube::Client;
use serde_json::Value;

use k8s_openapi::api::core::v1::ContainerPort;

use crate::controller::Error;
use crate::crd::{HelperSpec, WebhookHelper};
use crate::operator;

pub async fn bootstrap(namespace: String) -> Result<(), Error> {
    let container_port = 9443;
    let client = Client::try_default().await?;

    let mut label_map: BTreeMap<String, String> = BTreeMap::new();
    label_map.insert("app".to_string(), "webhook-helper".to_string());

    let deployment = Deployment {
        metadata: ObjectMeta {
            name: Some("webhook-helper".to_string()),
            namespace: Some(namespace.clone()),
            labels: Some(label_map.clone()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(label_map.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        args: Some(vec![
                            "run".into(),
                            "-p".into(),
                            format!("{}", container_port),
                        ]),
                        #[cfg(feature = "local")]
                        image: Some("webhook-helper:latest".to_string()),
                        #[cfg(not(feature = "local"))]
                        image: Some("rc1405/webhook-helper:latest".to_string()),
                        ports: Some(vec![ContainerPort {
                            container_port,
                            protocol: Some("TCP".into()),
                            ..Default::default()
                        }]),
                        name: String::from("webhook-helper"),
                        #[cfg(feature = "local")]
                        image_pull_policy: Some("Never".into()),
                        ..Default::default()
                    }],
                    service_account_name: Some("webhook-helper-service-account".into()),
                    ..Default::default()
                }),
            },
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(label_map),
                ..Default::default()
            },
            ..Default::default()
        }),
        status: None,
    };

    let deployment_value: Value = serde_json::to_value(deployment)?;

    let webhook = ValidatingWebhookConfiguration {
        metadata: ObjectMeta {
            name: Some("webhook-helper-admission".to_string()),
            ..Default::default()
        },
        webhooks: Some(vec![ValidatingWebhook {
            admission_review_versions: vec!["v1".to_string()],
            client_config: WebhookClientConfig {
                ..Default::default()
            },
            failure_policy: Some("Fail".to_string()),
            name: format!("webhook-helper.{}.svc", namespace.to_lowercase()),
            rules: Some(vec![RuleWithOperations {
                api_groups: Some(vec!["webhook-helper.io".to_string()]),
                api_versions: Some(vec!["v1".to_string()]),
                operations: Some(vec!["CREATE".to_string(), "UPDATE".to_string()]),
                resources: Some(vec!["webhook-helpers".to_string()]),
                ..Default::default()
            }]),
            side_effects: "None".to_string(),
            timeout_seconds: Some(15),
            ..Default::default()
        }]),
    };

    let webhook_value: Value = serde_json::to_value(webhook)?;

    operator::bootstrap::bootstrap(
        client.clone(),
        WebhookHelper {
            metadata: ObjectMeta {
                name: Some(format!("webhook-helper.{}.svc", namespace.to_lowercase())),
                namespace: Some(namespace.clone()),
                ..Default::default()
            },
            spec: HelperSpec {
                namespace: namespace.clone(),
                webhook: webhook_value,
                listening_port: container_port,
                target_port: Some(container_port),
                path: Some("/validate".into()),
                deployment: deployment_value,
                container_name: Some("webhook-helper".into()),
            },
            status: None,
        },
    )
    .await?;
    Ok(())
}
