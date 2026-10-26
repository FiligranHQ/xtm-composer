use crate::config::settings::Registry;
use base64::Engine;
use base64::engine::general_purpose;
use bollard::auth::DockerCredentials;
use serde::Serialize;
use slug::slugify;
use std::collections::{BTreeMap, HashMap};

pub struct Image {
    config: Registry,
    tag_suffix: Option<String>,
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
    pub fn new(config: Option<Registry>, tag_suffix: Option<String>) -> Self {
        Self {
            config: config.unwrap_or(Registry {
                server: None,
                username: None,
                password: None,
                email: None,
            }),
            tag_suffix,
        }
    }

    // region Docker
    pub fn build_name(&self, image_name: String) -> String {
        let image_name = self.with_tag_suffix(image_name);
        match self.config.server {
            None => image_name,
            Some(_) => format!("{}/{}", self.config.server.as_ref().unwrap(), image_name),
        }
    }

    // 6.9.0 -> 6.9.0-fips
    fn with_tag_suffix(&self, image_name: String) -> String {
        let suffix = match self.tag_suffix.as_deref() {
            Some(suffix) if !suffix.is_empty() => suffix,
            _ => return image_name,
        };
        // No tag to change on a digest
        if image_name.contains('@') {
            return image_name;
        }
        // Skip the registry port
        let name_start = image_name.rfind('/').map_or(0, |index| index + 1);
        match image_name[name_start..].rfind(':') {
            Some(index) if image_name[name_start + index + 1..].ends_with(suffix) => image_name,
            Some(_) => format!("{}{}", image_name, suffix),
            None => format!("{}:latest{}", image_name, suffix),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(server: Option<&str>) -> Option<Registry> {
        Some(Registry {
            server: server.map(str::to_string),
            username: None,
            password: None,
            email: None,
        })
    }

    fn build_name(server: Option<&str>, tag_suffix: Option<&str>, image_name: &str) -> String {
        Image::new(registry(server), tag_suffix.map(str::to_string))
            .build_name(image_name.to_string())
    }

    #[test]
    fn build_name_keeps_the_image_without_suffix() {
        assert_eq!(
            build_name(None, None, "opencti/connector-mitre:6.9.0"),
            "opencti/connector-mitre:6.9.0"
        );
        assert_eq!(
            build_name(None, Some(""), "opencti/connector-mitre:6.9.0"),
            "opencti/connector-mitre:6.9.0"
        );
    }

    #[test]
    fn build_name_appends_the_suffix_to_the_tag() {
        assert_eq!(
            build_name(None, Some("-fips"), "opencti/connector-mitre:6.9.0"),
            "opencti/connector-mitre:6.9.0-fips"
        );
        assert_eq!(
            build_name(None, Some("-ubi9"), "openaev/injector-nmap:2.1.0"),
            "openaev/injector-nmap:2.1.0-ubi9"
        );
    }

    #[test]
    fn build_name_appends_the_suffix_with_a_registry_server() {
        assert_eq!(
            build_name(
                Some("registry.local:5000"),
                Some("-fips"),
                "opencti/connector-mitre:6.9.0"
            ),
            "registry.local:5000/opencti/connector-mitre:6.9.0-fips"
        );
    }

    #[test]
    fn build_name_suffixes_latest_for_an_image_without_tag() {
        assert_eq!(
            build_name(None, Some("-fips"), "opencti/connector-mitre"),
            "opencti/connector-mitre:latest-fips"
        );
        assert_eq!(
            build_name(
                None,
                Some("-fips"),
                "localhost:5000/opencti/connector-mitre"
            ),
            "localhost:5000/opencti/connector-mitre:latest-fips"
        );
    }

    #[test]
    fn build_name_does_not_suffix_twice() {
        assert_eq!(
            build_name(None, Some("-fips"), "opencti/connector-mitre:6.9.0-fips"),
            "opencti/connector-mitre:6.9.0-fips"
        );
    }

    #[test]
    fn build_name_keeps_an_image_pinned_by_digest() {
        let image = "opencti/connector-mitre:6.9.0@sha256:0123456789abcdef";
        assert_eq!(build_name(None, Some("-fips"), image), image);
    }
}
