# XTM Composer

The following repository is used to store the platform XTM composer.
The composer allows [OpenCTI](https://github.com/OpenCTI-Platform/opencti) and [OpenAEV](https://github.com/OpenAEV-Platform/openaev) users to manage their connectors/collectors/injectors directly from the platform.
For performance and low level access, the agent is written in Rust. Please start your journey
with https://doc.rust-lang.org/book.

## Documentation


- [Architecture Design](https://docs.opencti.io/latest/deployment/integration-manager/architecture/) - Detailed technical architecure documentation
- [Installation Guide](https://docs.opencti.io/latest/deployment/integration-manager/installation/) - System requirements and installation methods
- [Quick Start](https://docs.opencti.io/latest/deployment/integration-manager/quick-start/) - Get up and running quickly
- [Configuration Reference](https://docs.opencti.io/latest/deployment/integration-manager/configuration/) - Complete configuration documentation
- [Development Guide](https://docs.opencti.io/latest/development/integration-manager/) - Setup for development and contribution

## Module Status

- **OpenCTI**: ✅ Fully implemented and production-ready
  - Compatible version starts at 6.8.0
- **OpenAEV**: 🚧 Coming Soon

## Connector types

XTM Composer does not depend on the connector type. It deploys the image of the contract with the contract
configuration (sensitive values decrypted with the manager private key), adds `OPENCTI_URL` and `OPENCTI_CONFIG_HASH`,
and reports the status, logs and health of the container. Every value of the OpenCTI `ConnectorType` enumeration is
deployed the same way: `EXTERNAL_IMPORT`, `INTERNAL_IMPORT_FILE`, `INTERNAL_ENRICHMENT`, `INTERNAL_ANALYSIS`,
`INTERNAL_EXPORT_FILE`, `INTERNAL_HUNT` and `STREAM`.

Internal hunt connectors (`INTERNAL_HUNT`) execute OpenCTI hunts on one hunted platform, such as Splunk, Microsoft
Sentinel or Elastic Security. Before deploying one, make sure that:

- the container can reach the API of the hunted platform in addition to OpenCTI;
- the platform credentials only grant read-only search permissions; they are entered in the OpenCTI catalog form and
  reach XTM Composer encrypted, like every sensitive value;
- the OpenCTI platform provides hunts: a hunt connector stops at start when the platform or its pycti does not know the
  type, which shows as a restart loop in the connector health.

## Orchestration

Composer act as a micro orchestration tool to interface Filigran product to different major container orchestration
systems.
Every type of orchestration must implement the trait Orchestrator to be fully supported.
If your system is not in this list, please create a feature request.

### Kubernetes

Kubernetes, also known as K8s, is an open source system for automating deployment, scaling, and management of
containerized applications. https://kubernetes.io/

> We recommend kubernetes for you Filigran deployments

### Portainer

Portainer is a universal container management platform. You can manage environments of any type, anywhere (Docker and
Kubernetes, running on dev laptops, in your DC, in the cloud, or at the edge), and we don't require you to run any
specific Kubernetes distro. https://www.portainer.io/

> Only docker through portainer is currently supported using a direct socket binding

### Docker

Docker is a set of platform as a service (PaaS) products that use OS-level virtualization to deliver software in
packages called containers. https://www.docker.com/
If you don't have any orchestration system and you use direct docker-composer, its the mode you need

> Direct docker daemon access require also a direct socket binding

## About

XTM composer is a product designed and developed by the company [Filigran](https://filigran.io).

<a href="https://filigran.io" alt="Filigran"><img src="https://github.com/OpenCTI-Platform/opencti/raw/master/.github/img/logo_filigran.png" width="300" /></a>

## Release

1. Change version in cargo.toml (ex:1.0.0)
2. Push that version on master
3. Create a git tag with same numbers (ex:1.0.0)
4. Push a git tag in the format X.X.X to the master branch. The Docker image will be built with the same tag.
