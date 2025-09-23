# OpenCTI GraphQL Queries for Connector Management

This document contains the GraphQL queries and mutations used by XTM Composer for managing connectors in OpenCTI.

## GraphQL Types

### Enums

```graphql
enum ConnectorRequestStatus {
  starting
  stopping
}

enum ConnectorCurrentStatus {
  started
  stopped
}
```

### Object Types

```graphql
type ManagerContractConfiguration {
  key: String
  value: String
}
```

## Queries

### Get OpenCTI Version

```graphql
query getVersion {
  about {
    version
  }
}
```

**Type Definition:**
```graphql
type AppInfo {
  version: String!
}
```

### Get Catalogs

```graphql
query getCatalogs {
  catalogs {
    id
    name
    description
    contracts
  }
}
```

**Type Definition:**
```graphql
type Catalog {
  id: ID!
  name: String!
  description: String
  contracts: [String]
}
```

**Description:**
Returns the list of available catalogs and their connector contracts in OpenCTI.

### List Connectors for Managers

```graphql
query connectorsForManagers {
  connectorsForManagers {
    id
    standard_id
    name
    connector_type
    connector_scope
    connector_state
    connector_user_id
    catalog_id
    manager_id
    manager_contract_image
    manager_requested_status
    manager_current_status
    manager_connector_logs
    manager_contract_hash
    manager_contract_configuration {
      key
      value
    }
  }
}
```

**Type Definition:**
```graphql
type ManagedConnector {
  id: ID!
  standard_id: String!
  name: String!
  connector_type: String
  connector_scope: [String]
  connector_state: String
  connector_user_id: String
  catalog_id: String
  manager_id: String
  manager_contract_image: String
  manager_requested_status: ConnectorRequestStatus
  manager_current_status: ConnectorCurrentStatus
  manager_connector_logs: String
  manager_contract_hash: String
  manager_contract_configuration: [ManagerContractConfiguration]
}
```

## Mutations

### Register Connector Manager

```graphql
mutation registerConnectorsManager($input: RegisterConnectorsManagerInput!) {
  registerConnectorsManager(input: $input) {
    id
    name
    about_version
  }
}
```

**Input Type Definition:**
```graphql
input RegisterConnectorsManagerInput {
  id: ID!
  name: String!
  public_key: String!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "8215614c-7139-422e-b825-b20fd2a13a23",
    "name": "OpenCTI XTM Composer",
    "public_key": "-----BEGIN RSA PUBLIC KEY-----\n...\n-----END RSA PUBLIC KEY-----"
  }
}
```

### Add Managed Connector

```graphql
mutation addManagedConnector($input: AddManagedConnectorInput!) {
  managedConnectorAdd(input: $input) {
    id
    standard_id
    name
    connector_type
    connector_user_id
    catalog_id
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

**Input Type Definition:**
```graphql
input AddManagedConnectorInput {
  name: String!
  connector_user_id: ID
  user_id: ID!
  catalog_id: ID!
  automatic_user: Boolean
  confidence_level: String
  manager_contract_image: String!
  manager_contract_configuration: [ContractConfigInput!]!
}

input ContractConfigInput {
  key: String!
  value: String!
}
```

**Variables Example:**
```json
{
  "input": {
    "name": "IpInfo Enrichment Connector",
    "user_id": "88ec0c6a-13ce-5e39-b486-354fe4a7084f",
    "connector_user_id": "88ec0c6a-13ce-5e39-b486-354fe4a7084f",
    "catalog_id": "catalog-ipinfo-id",
    "automatic_user": false,
    "manager_contract_image": "opencti/connector-ipinfo:latest",
    "manager_contract_configuration": [
      { "key": "IPINFO_TOKEN", "value": "your-token-here" },
      { "key": "IPINFO_MAX_TLP", "value": "TLP:AMBER" },
      { "key": "IPINFO_USE_ASN_NAME", "value": "false" },
      { "key": "CONNECTOR_SCOPE", "value": "IPv4-Addr" },
      { "key": "CONNECTOR_AUTO", "value": "true" }
    ]
  }
}
```

### Update Manager Status (Ping)

```graphql
mutation updateManagerStatus($input: UpdateConnectorManagerStatusInput!) {
  updateConnectorManagerStatus(input: $input) {
    id
    name
    about_version
  }
}
```

**Input Type Definition:**
```graphql
input UpdateConnectorManagerStatusInput {
  id: ID!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "8215614c-7139-422e-b825-b20fd2a13a23"
  }
}
```

### Update Connector Current Status

```graphql
mutation updateConnectorStatus($input: CurrentConnectorStatusInput!) {
  updateConnectorCurrentStatus(input: $input) {
    id
    name
    manager_id
    manager_requested_status
    manager_current_status
  }
}
```

**Input Type Definition:**
```graphql
input CurrentConnectorStatusInput {
  id: ID!
  status: ConnectorCurrentStatus!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "51f4ba89-d9b3-483f-ad62-4d1a326ea25a",
    "status": "started"
  }
}
```

### Update Connector Requested Status

```graphql
mutation updateRequestedStatus($input: RequestConnectorStatusInput!) {
  updateConnectorRequestedStatus(input: $input) {
    id
    name
    manager_current_status
    manager_requested_status
  }
}
```

**Input Type Definition:**
```graphql
input RequestConnectorStatusInput {
  id: ID!
  status: ConnectorRequestStatus!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "90ceb3d0-6663-497c-b82f-1804baf52685",
    "status": "starting"
  }
}
```

### Report Connector Logs

```graphql
mutation reportConnectorLogs($input: LogsConnectorStatusInput!) {
  updateConnectorLogs(input: $input)
}
```

**Input Type Definition:**
```graphql
input LogsConnectorStatusInput {
  id: ID!
  logs: [String!]!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "51f4ba89-d9b3-483f-ad62-4d1a326ea25a",
    "logs": [
      "[INFO] Connector started successfully",
      "[INFO] Processing entity: report-123",
      "[WARN] Rate limit reached, waiting 60 seconds"
    ]
  }
}
```

### Report Connector Health

```graphql
mutation reportConnectorHealth($input: HealthConnectorStatusInput!) {
  updateConnectorHealth(input: $input)
}
```

**Input Type Definition:**
```graphql
input HealthConnectorStatusInput {
  id: ID!
  restart_count: Int!
  started_at: DateTime!
  is_in_reboot_loop: Boolean!
}
```

**Variables Example:**
```json
{
  "input": {
    "id": "51f4ba89-d9b3-483f-ad62-4d1a326ea25a",
    "restart_count": 0,
    "started_at": "2025-01-19T16:27:31Z",
    "is_in_reboot_loop": false
  }
}
```

### Delete Connector

```graphql
mutation deleteConnector($id: ID!) {
  deleteConnector(id: $id)
}
```

**Variables Example:**
```json
{
  "id": "64d49217-b512-4689-bc4c-b7cac60f94f4"
}
```

## Status Values

### ConnectorCurrentStatus
- `started` - Connector is running
- `stopped` - Connector is stopped

### ConnectorRequestStatus  
- `starting` - Request to start the connector
- `stopping` - Request to stop the connector

## Usage Notes

1. All mutations require authentication via Bearer token in the Authorization header
2. The `manager_id` should match the configured XTM Composer manager ID
3. The `connector_user_id` is the OpenCTI user ID that will own the connector
4. Configuration values are encrypted using the manager's public key
5. Logs are sent as an array of strings and stored for debugging
6. Health metrics help track connector stability and restart patterns

## Error Handling

If OpenCTI doesn't support XTM Composer operations, the following features will gracefully degrade:
- Version query may return null
- Manager registration will log a warning but continue
- Status updates will be ignored but connectors will still run
- Log reporting will be skipped
- Health metrics won't be tracked

The XTM Composer will continue to operate even if some GraphQL operations fail, ensuring resilience in different OpenCTI deployments.
