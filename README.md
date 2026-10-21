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

### Podman

Podman is a daemonless container engine, the standard one of RHEL and SUSE Linux Enterprise. https://podman.io/
Connectors are deployed through the Podman API socket, in rootful or rootless mode, with the `podman` selector.

> See the [Podman guide](dev/podman/README.md): socket setup, rootless mode, private registries and a compose example

## About

XTM composer is a product designed and developed by the company [Filigran](https://filigran.io).

<a href="https://filigran.io" alt="Filigran"><img src="https://github.com/OpenCTI-Platform/opencti/raw/master/.github/img/logo_filigran.png" width="300" /></a>

## Release

1. Bump the version in `Cargo.toml` and `Cargo.lock` (ex: 1.0.0) through a pull request merged into master.
2. Create the GitHub release with the same version, targeting master, with generated release notes:

   ```bash
   gh release create 1.0.0 --target master --generate-notes
   ```

   This creates the `1.0.0` tag, which builds and publishes the Docker images with the same tag.
   The release can also be created from the GitHub Releases page, with a new tag targeting master.

   To release from a signed tag instead, push the tag first, then create the release on it:

   ```bash
   git tag -s 1.0.0 -m 1.0.0
   git push origin 1.0.0
   gh release create 1.0.0 --verify-tag --generate-notes
   ```
