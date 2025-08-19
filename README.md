# XTM composer

The following repository is used to store the platform XTM composer.
The composer allows OpenCTI / OpenBAS users to manager their connectors/collectors/injectors directly from the platform
For performance and low level access, the agent is written in Rust. Please start your journey
with https://doc.rust-lang.org/book.

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

Push a tag with a format X.X.X on master branch: the docker image is build with this tag too.