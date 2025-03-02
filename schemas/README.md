# Useful queries

```text
query connectors {
    connectors {
        id
        name
        auto
        active
        configurations {
            name
        }
    }
}

query getManagers {
    connectorManagers {
        id
        name
        connector_manager_contracts
    }
}

query connectorsForManager {
    connectorsForManager(managerId: "8215614c-7139-422e-b825-b20fd2a13a23") {
        id
        manager_id
        manager_contract_image
        manager_requested_status
        manager_current_status
        manager_connector_logs

        manager_contract_configuration {
            key
            value
        }
    }
}

mutation reportConnectorLogs {
    updateConnectorLogs(input: {
        id: "51f4ba89-d9b3-483f-ad62-4d1a326ea25a"
        logs: ["log1", "log2"]
    }) {
        id
        manager_id
        manager_contract_image
        manager_requested_status
        manager_current_status
        manager_connector_logs
        manager_contract_configuration {
            key
            value
        }
    }
}

mutation delete {
    deleteConnector(id: "64d49217-b512-4689-bc4c-b7cac60f94f4")
}

mutation updateRequestedStatus {
    updateConnectorRequestedStatus(input: {
        id: "90ceb3d0-6663-497c-b82f-1804baf52685",
        status: starting
    }) {
        id
        manager_current_status
        manager_requested_status
    }
}

mutation registerConnectorsManager {
    registerConnectorsManager(input: {
        id: "8215614c-7139-422e-b825-b20fd2a13a23"
        name: "OpenCTI Composer"
        contracts: ["{\r\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\r\n  \"$id\": \"https://www.filigran.io/mitre.schema.json\",\r\n  \"title\": \"IpInfo connector\",\r\n  \"description\": \"IpInfo enrichment connector\",\r\n  \"labels\": [\"enrichment\", \"ip\"],\r\n  \"container_image\": \"opencti/connector-ipinfo\",\r\n  \"container_type\": \"INTERNAL_ENRICHMENT\",\r\n  \"type\": \"object\",\r\n  \"default\": {\r\n    \"CONNECTOR_SCOPE\": \"IPv4-Addr\",\r\n    \"CONNECTOR_AUTO\": true,\r\n    \"IPINFO_MAX_TLP\": \"TLP:AMBER\",\r\n    \"IPINFO_USE_ASN_NAME\": false\r\n  },\r\n  \"properties\": {\r\n    \"CONNECTOR_SCOPE\": {\r\n      \"description\": \"Scope\",\r\n      \"type\": \"string\"\r\n    },\r\n    \"CONNECTOR_AUTO\": {\r\n      \"description\": \"Auto trigger\",\r\n      \"type\": \"boolean\"\r\n    },\r\n    \"IPINFO_TOKEN\": {\r\n      \"description\": \"Token\",\r\n      \"type\": \"string\",\r\n      \"format\": \"password\"\r\n    },\r\n    \"IPINFO_MAX_TLP\": {\r\n      \"description\": \"Max TLP\",\r\n      \"type\": \"string\"\r\n    },\r\n    \"IPINFO_USE_ASN_NAME\": {\r\n      \"description\": \"use ASN name\",\r\n      \"type\": \"boolean\"\r\n    }\r\n  },\r\n  \"required\": [\"IPINFO_TOKEN\"],\r\n  \"additionalProperties\": false\r\n}"]
    }) {
        id
        connector_manager_contracts
    }
}

mutation registerManagedConnector {
    managedConnectorAdd(input: {
        name: "Hardcoded connector for manager",
        manager_id: "8215614c-7139-422e-b825-b20fd2a13a23",
        connector_user_id: "88ec0c6a-13ce-5e39-b486-354fe4a7084f",
        manager_contract_image: "opencti/connector-ipinfo",
        manager_contract_configuration: [
            { key: "IPINFO_TOKEN", value: "4f0b8a3ffc13d8" },
            { key: "IPINFO_MAX_TLP", value: "TLP:AMBER" },
            { key: "IPINFO_USE_ASN_NAME", value: "false" },
        ]
    }) {
        id
        manager_id
        manager_contract_image
        manager_contract_hash
        manager_requested_status
        manager_current_status
        manager_contract_configuration {
            key
            value
        }
    }
}
```