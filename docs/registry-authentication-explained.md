# Docker Registry Authentication

> How XTM Composer handles Docker registry authentication for connector deployments

## Overview

XTM Composer supports pulling connector images from custom Docker registries. Authentication is **optional** and uses a fallback chain when credentials aren't configured.

## Quick Reference

| Registry Type | Configuration Required | Result |
|--------------|------------------------|---------|
| Public Docker Hub | None | ✅ Works automatically |
| Public registry | Registry URL only | ✅ Works automatically |
| Private registry (unauthenticated) | Registry URL only | ❌ Fails with auth error |
| Private registry | URL + credentials | ✅ Works with provided credentials |

### Authentication Fallback Chain

```
1. XTM Composer config credentials (if configured)
   ↓
2. Docker daemon credentials (~/.docker/config.json)
   ↓
3. Docker credential store (OS keychain)
   ↓
4. Anonymous/public access attempt
   ↓
5. Authentication error (if registry requires auth)
```

## Configuration Examples

### Public Registry (No Authentication)

```yaml
opencti:
  daemon:
    docker:
      registry:
        url: localhost:5000
```

XTM Composer attempts public access. If this fails, Docker falls back to daemon credentials.

### Private Registry (With Authentication)

```yaml
opencti:
  daemon:
    docker:
      registry:
        url: registry.company.com:5000
        username: your-username
        password: your-password
        insecure: false  # Optional: allow HTTP connections
```

XTM Composer uses the provided credentials for authentication.

### Default Behavior (No Registry Config)

```yaml
opencti:
  daemon:
    docker:
      # No registry configuration
```

Uses Docker Hub and daemon's default authentication.

## Implementation Details

### Image Name Resolution

The `get_full_image_name()` method prepends the configured registry URL to image names:

**Examples:**
```
Config: registry.url = "localhost:5000"

Input                          → Output
─────────────────────────────────────────────────────────
connector-misp:5.0.0          → localhost:5000/connector-misp:5.0.0
myorg/connector:1.0           → localhost:5000/myorg/connector:1.0
docker.io/alpine:3.18         → docker.io/alpine:3.18 (unchanged)
registry.com/app:v1           → registry.com/app:v1 (unchanged)
```

**Detection logic:** An image is considered to have a registry if it contains `/` and the first part contains `.`

### Authentication Building

The `build_registry_auth()` method constructs authentication credentials:

```rust
fn build_registry_auth(&self) -> Option<DockerCredentials> {
    let settings = crate::settings();
    let docker_config = settings.opencti.daemon.docker.as_ref()?;
    let registry_config = docker_config.registry.as_ref()?;

    Some(DockerCredentials {
        username: registry_config.username.clone(),  // Can be None
        password: registry_config.password.clone(),  // Can be None
        ..Default::default()
    })
}
```

**Returns:**
- `Some(DockerCredentials)` if registry config exists (credentials optional)
- `None` if no registry configuration found

### Deployment Flow

```rust
async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
    // 1. Build full image name with registry prefix
    let full_image_name = self.get_full_image_name(&connector.image);

    // 2. Build authentication credentials
    let auth = self.build_registry_auth();

    // 3. Pull image from registry
    self.docker.create_image(
        Some(CreateImageOptions {
            from_image: full_image_name.as_str(),
            ..Default::default()
        }),
        None,
        auth,  // Optional: Docker handles None gracefully
    ).await;

    // 4. Create and start container
    // ...
}
```

## Authentication Scenarios

### 1. Public Docker Hub (Default)

**Configuration:** None

**Behavior:**
- Image: `alpine:3.18` → Pulls from `docker.io/library/alpine:3.18`
- Auth: None
- Result: ✅ Success (public access)

### 2. Public Custom Registry

**Configuration:**
```yaml
registry:
  url: localhost:5000
```

**Behavior:**
- Image: `connector-misp:5.0.0` → Pulls from `localhost:5000/connector-misp:5.0.0`
- Auth: Attempts anonymous access
- Result: ✅ Success (if registry allows) / ❌ Fails (if auth required)

### 3. Private Registry with Credentials

**Configuration:**
```yaml
registry:
  url: registry.company.com
  username: myuser
  password: mypassword
```

**Behavior:**
- Image: `connector-misp:5.0.0` → Pulls from `registry.company.com/connector-misp:5.0.0`
- Auth: Uses provided username and password
- Result: ✅ Success (correct creds) / ❌ Fails (incorrect creds)

### 4. Registry with Docker Daemon Credentials

**Configuration:**
```yaml
registry:
  url: localhost:5000
```

**Docker Config (~/.docker/config.json):**
```json
{
  "auths": {
    "localhost:5000": {
      "auth": "base64credentials"
    }
  }
}
```

**Behavior:**
- Image: `connector-misp:5.0.0` → Pulls from `localhost:5000/connector-misp:5.0.0`
- Auth: XTM Composer sends `None`, Docker falls back to daemon credentials
- Result: ✅ Success (uses daemon credentials)

## Rust Implementation Notes

### Early Return with `?` Operator

```rust
let registry_config = docker_config.registry.as_ref()?;
```

Returns `None` immediately if `registry` is not configured. Equivalent to:
```rust
match docker_config.registry.as_ref() {
    Some(config) => config,
    None => return None,
}
```

### Optional Credentials

```rust
username: registry_config.username.clone()  // Type: Option<String>
```

- Configured: `Some("myuser")`
- Not configured: `None`

Docker handles `None` by using fallback authentication (daemon credentials or public access).

### Default Field Values

```rust
DockerCredentials {
    username: registry_config.username.clone(),
    password: registry_config.password.clone(),
    ..Default::default()  // Sets remaining fields to None
}
```

Sets `serveraddress`, `email`, and `identitytoken` to `None`.

## Troubleshooting

### Enable Debug Logging

```yaml
manager:
  logger:
    level: debug
```

Look for deployment logs showing authentication details:
```
INFO Deploying container - Registry: localhost:5000, Original image: connector-misp:5.0.0, Pulling: localhost:5000/connector-misp:5.0.0
```

### Test Authentication Manually

```bash
# Login to your registry
docker login localhost:5000 -u myuser -p 'mypassword'

# Test pulling the image
docker pull localhost:5000/connector-misp:5.0.0

# If successful, XTM Composer should work with the same credentials
```

### Common Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `authentication required` | Private registry without credentials | Add `username` and `password` to config |
| `unauthorized` | Wrong credentials | Verify credentials with `docker login` |
| `connection refused` | Registry not accessible | Check registry URL and network connectivity |
| `x509: certificate signed by unknown authority` | Self-signed certificate | Set `insecure: true` (not recommended for production) |

## Changes Summary

### Modified Files

| File | Changes |
|------|---------|
| `src/config/settings.rs` | Added `Registry` struct with `url`, `username`, `password`, `insecure` fields |
| `src/orchestrator/docker/docker.rs` | Added `get_full_image_name()` and `build_registry_auth()` methods; updated `deploy()` to use registry configuration |
| `config/default.yaml` | Added registry configuration example under `opencti.daemon.docker` |

### Key Features

- ✅ **Optional authentication** - Works with public and private registries
- ✅ **Fallback chain** - Uses daemon credentials if not configured
- ✅ **Automatic image name resolution** - Prepends registry URL intelligently
- ✅ **Secure credential handling** - Supports username/password authentication
- ✅ **Documented code** - Inline comments and Rust doc comments

---

**Last Updated:** 2025-12-20
