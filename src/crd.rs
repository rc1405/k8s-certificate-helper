use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub enum Stage {
    Deleting,
    Creating,
    CertificateCreated(String),
    CreationFailed(String),
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let message: String = match self {
            Stage::CertificateCreated(_) => "CertificateCreated".into(),
            Stage::CreationFailed(_) => "CreationFailed".into(),
            Stage::Deleting => "Deleting".into(),
            Stage::Creating => "Creating".into(),
        };
        write!(f, "{}", message)
    }
}

impl Stage {
    pub fn message(&self) -> String {
        match self {
            Stage::CertificateCreated(c) => format!("Certificate {} Created", c),
            Stage::CreationFailed(r) => format!("Webhook-helper failed to created webhook: {}", r),
            Stage::Deleting => "Deleting resource".into(),
            Stage::Creating => "Creating resource".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema, Default)]
pub struct CertificateCondition {
    #[serde(rename = "type")]
    pub type__: String,
    pub message: String,
    pub status: String,
    #[serde(rename = "lastTransitionTime")]
    pub last_transition_time: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema, Default)]
pub struct CertificateStatus {
    pub certificate: Option<String>,
    pub service: Option<String>,
    pub alt_names: Option<Vec<String>>,
    pub conditions: Option<Vec<CertificateCondition>>,
}

#[derive(CustomResource, Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema)]
#[kube(group = "certificate-helper.io", version = "v1", kind = "Certificate")]
#[kube(singular = "certificate", plural = "certificates")]
#[kube(status = "CertificateStatus")]
pub struct CertificateSpec {
    pub namespace: String,
    pub service: String,
    pub alt_names: Option<Vec<String>>,
}
