use crate::config::settings::Registry;
use bollard::auth::DockerCredentials;

pub struct Image {
    config: Registry,
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
}
