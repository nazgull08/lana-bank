pub mod error;

use error::ServiceAccountError;
use gcp_bigquery_client::yup_oauth2::ServiceAccountKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceAccountConfig {
    #[serde(skip)]
    pub gcp_project: String,
    #[serde(skip)]
    pub sa_creds_base64: Option<String>,
    #[serde(skip)]
    service_account_key: Option<ServiceAccountKey>,

    #[serde(default = "default_gcp_location")]
    pub gcp_location: String,
}

impl Default for ServiceAccountConfig {
    fn default() -> Self {
        Self {
            gcp_project: "".to_string(),
            sa_creds_base64: None,
            service_account_key: None,
            gcp_location: default_gcp_location(),
        }
    }
}

impl ServiceAccountConfig {
    pub fn set_sa_creds_base64(
        mut self,
        sa_creds_base64: Option<String>,
    ) -> Result<Self, ServiceAccountError> {
        if sa_creds_base64.is_none() {
            return Ok(self);
        }

        self.sa_creds_base64 = sa_creds_base64;

        let creds = self.get_json_creds()?;
        let service_account_key = serde_json::from_str::<ServiceAccountKey>(&creds)?;
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS_JSON", creds) };

        self.gcp_project = service_account_key
            .project_id
            .clone()
            .ok_or(ServiceAccountError::GCPProjectIdMissing)?;
        self.service_account_key = Some(service_account_key);

        Ok(self)
    }

    pub fn service_account_key(&self) -> Result<ServiceAccountKey, ServiceAccountError> {
        self.service_account_key
            .as_ref()
            .cloned()
            .ok_or(ServiceAccountError::CredentialsNotProvided)
    }

    fn get_json_creds(&self) -> Result<String, ServiceAccountError> {
        let creds = self
            .sa_creds_base64
            .as_ref()
            .ok_or(ServiceAccountError::CredentialsNotProvided)?
            .as_bytes();

        use base64::{Engine as _, engine::general_purpose};

        Ok(std::str::from_utf8(&general_purpose::STANDARD.decode(creds)?)?.to_string())
    }
}

fn default_gcp_location() -> String {
    "europe-west6".to_string()
}
