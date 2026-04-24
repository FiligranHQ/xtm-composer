use std::collections::HashMap;

#[derive(Debug, Clone)]
struct ApiConnector {
    id: String,
    platform: String,
}

trait Orchestrator {
    fn labels(&self, connector: &ApiConnector) -> HashMap<String, String> {
        let mut labels = HashMap::new();
        labels.insert("opencti-manager".to_string(), "shared-manager".to_string());
        labels.insert("opencti-connector-id".to_string(), connector.id.clone());
        labels.insert("opencti-platform".to_string(), connector.platform.clone());
        labels
    }
}

struct FakeOrchestrator;

impl Orchestrator for FakeOrchestrator {}

#[test]
fn labels_include_platform_discriminator() {
    let connector = ApiConnector {
        id: "connector-1".to_string(),
        platform: "opencti".to_string(),
    };
    let orchestrator = FakeOrchestrator;

    let labels = orchestrator.labels(&connector);

    assert_eq!(labels.get("opencti-connector-id"), Some(&connector.id));
    assert_eq!(labels.get("opencti-platform"), Some(&connector.platform));
}

