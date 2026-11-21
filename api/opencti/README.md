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

enum ConnectorType {
  EXTERNAL_IMPORT
  INTERNAL_IMPORT_FILE
  INTERNAL_ENRICHMENT
  INTERNAL_ANALYSIS
  INTERNAL_EXPORT_FILE
  INTERNAL_HUNT
  STREAM
}
```

`ConnectorType` mirrors the OpenCTI enumeration. `ManagedConnector` has no connector type field: the type reaches XTM
Composer as the `CONNECTOR_TYPE` entry of `manager_contract_configuration` (for example
`{ "key": "CONNECTOR_TYPE", "value": "INTERNAL_HUNT", "encrypted": false }`), and XTM Composer passes it to the
container environment unchanged without reading it. Every connector type, including the internal hunt connectors
(`INTERNAL_HUNT`), is therefore deployed the same way, from its contract image and configuration. When OpenCTI adds a
connector type, add it to the schema copy `opencti.graphql`; the test
`schema_copy_declares_every_opencti_connector_type` pins the list.

### Object Types

```graphql
type ConnectorContractConfiguration {
  key: String!
  value: String
  encrypted: Boolean
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
  description: String!
  contracts: [String!]!
}
```

**Description:**
Returns the list of available catalogs and their connector contracts in OpenCTI.

### List Connectors for Managers

```graphql
query connectorsForManagers {
  connectorsForManagers {
    id
    name
    manager_contract_hash
    manager_contract_image
    manager_current_status
    manager_requested_status
    manager_contract_configuration {
      key
      value
      encrypted
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
  connector_user_id: ID
  connector_state_timestamp: DateTime
  manager_contract_image: String!
  manager_current_status: String
  manager_requested_status: String!
  manager_contract_configuration: [ConnectorContractConfiguration!]!
  manager_contract_hash: String!
  manager_connector_logs: [String!]!
  manager_health_metrics: ConnectorHealthMetrics
}
```

The connector type, the scope and every other setting of the connector arrive as entries of
`manager_contract_configuration` (`CONNECTOR_TYPE`, `CONNECTOR_SCOPE`, ...). Entries with `encrypted: true` are
decrypted with the manager private key and stay flagged as sensitive in the container definition.

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
    connector_user_id
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
  connector_user_id: ID!
  catalog_id: ID!
  manager_contract_image: String!
  manager_contract_configuration: [ContractConfigInput!]!
}

input ContractConfigInput {
  key: String!
  value: [String!]
}
```

**Variables Example:**
```json
{
  "input": {
    "name": "IpInfo Enrichment Connector",
    "connector_user_id": "88ec0c6a-13ce-5e39-b486-354fe4a7084f",
    "catalog_id": "catalog-ipinfo-id",
    "manager_contract_image": "opencti/connector-ipinfo:latest",
    "manager_contract_configuration": [
      { "key": "IPINFO_TOKEN", "value": ["your-token-here"] },
      { "key": "IPINFO_MAX_TLP", "value": ["TLP:AMBER"] },
      { "key": "IPINFO_USE_ASN_NAME", "value": ["false"] },
      { "key": "CONNECTOR_SCOPE", "value": ["IPv4-Addr"] },
      { "key": "CONNECTOR_AUTO", "value": ["true"] }
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
  started_at: String!
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
2. The `id` sent to `registerConnectorsManager` and `updateConnectorManagerStatus` is the configured XTM Composer
   manager ID
3. The `connector_user_id` is the OpenCTI user ID that will own the connector
4. Sensitive configuration values (`encrypted: true`) are encrypted using the manager's public key
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
