use crate::config::settings::Registry;
use base64::Engine;
use base64::engine::general_purpose;
use bollard::auth::DockerCredentials;
use serde::Serialize;
use slug::slugify;
use std::collections::{BTreeMap, HashMap};

pub struct Image {
    config: Registry,
}

#[derive(Serialize)]
struct DockerConfig {
    auths: HashMap<String, DockerAuthEntry>,
}

#[derive(Serialize)]
struct DockerAuthEntry {
    username: String,
    password: String,
    email: Option<String>,
    auth: String, // base64(user:pass)
}

impl Image {
    pub fn new(config: Option<Registry>) -> Self {
        Self {
            config: config.unwrap_or(Registry {
                server: None,
                username: None,
                password: None,
                email: None,
            }),
        }
    }

    // region Docker
    pub fn build_name(&self, image_name: String) -> String {
        match self.config.server {
            None => image_name,
            Some(_) => format!("{}/{}", self.config.server.as_ref().unwrap(), image_name),
        }
    }

    fn build_credentials(&self, config: &Registry) -> DockerCredentials {
        DockerCredentials {
            username: config.username.clone(),
            password: config.password.clone(),
            auth: None,
            email: config.email.clone(),
            serveraddress: config.server.clone(),
            identitytoken: None,
            registrytoken: None,
        }
    }

    pub fn get_credentials(&self) -> Option<DockerCredentials> {
        if self.config.username.is_none() || self.config.password.is_none() {
            return None;
        }
        Some(self.build_credentials(&self.config))
    }
    // endregion

    // region Kubernetes
    pub fn get_kubernetes_secret_name(&self) -> Option<String> {
        // secret name must be slug to be compatible with kubernetes naming convention (RFC 1123)
        self.config.server.clone().map(|server| slugify(&server))
    }

    pub fn get_kubernetes_registry_secret(&self) -> Option<BTreeMap<String, String>> {
        let registry_config = self.config.clone();
        if registry_config.username.is_some() && registry_config.password.is_some() {
            let username = registry_config.username?.clone();
            let password = registry_config.password?.clone();
            let auth_string = format!("{}:{}", username, password);
            let auth_encoded = general_purpose::STANDARD.encode(auth_string);
            let entry = DockerAuthEntry {
                username,
                password,
                email: registry_config.email.clone(),
                auth: auth_encoded,
            };
            let config = DockerConfig {
                auths: HashMap::from([(registry_config.server.unwrap().to_string(), entry)]),
            };
            Some(BTreeMap::from([(
                ".dockerconfigjson".to_string(),
                serde_json::to_string(&config).unwrap(),
            )]))
        } else {
            None
        }
    }
    // endregion
}
