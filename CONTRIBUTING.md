# Contributing to XTM Composer

Thank you for reading this documentation and considering making your contribution to the project. Any contribution that helps us improve XTM Composer is valuable and much appreciated. If it is also meaningful to you or your organisation it's all for the best.

In order to help you understand the project, where we are heading and how you can contribute, below are several resources and answers.

Do not hesitate to shoot us an [email](mailto:contact@opencti.io) or join us on our [Slack channel](https://community.filigran.io).


## Why contribute?

XTM Composer is an open source orchestration manager designed to streamline the deployment and management of OpenCTI connectors and other components. It provides a unified interface for managing complex deployments across different orchestration platforms (Docker, Kubernetes, Portainer).

Whether you are an organisation or an individual working with OpenCTI, contributing to the XTM Composer project may represent a great opportunity for you:

* You can help grow the OpenCTI ecosystem by improving the tools that make deployment and management easier and more efficient.

* You will be able to adapt the tool to your specific deployment needs and infrastructure requirements.

* XTM Composer offers an interesting opportunity for developers to work with modern technologies like Rust, container orchestration, and cloud-native deployments.


## Where is the project heading?

Our goals for future releases include:

* **Enhanced orchestration support**: Expanding support for more orchestration platforms and improving existing integrations.

* **Advanced deployment patterns**: Supporting complex deployment scenarios, auto-scaling, and high-availability configurations.

* **Better observability**: Improving monitoring, logging, and debugging capabilities for managed components.

* **OpenBAS integration**: Adding support for OpenBAS platform alongside OpenCTI.


## Code of Conduct

XTM Composer has adopted a Code of Conduct that we expect project participants to adhere to. Please read the [full text](CODE_OF_CONDUCT.md) so that you can understand which actions will and will not be tolerated.


## How can you contribute?

Any contribution is appreciated, and many don't imply coding. Contributions can range from a suggestion for improving documentation, requesting a new feature, reporting a bug, to developing features or fixing bugs yourself.

For general suggestions or questions about the project or the documentation, you can open an issue on the repository with the label "question". We will answer as soon as possible. If you do not wish to publish on the repository, please see the section below [**"How can you get in touch for other questions?"**](#how-can-you-get-in-touch-for-other-questions).

* **Testing and reporting issues**: Just using XTM Composer and opening issues if everything is not working as expected will be a huge step forward. To report a bug, please use our [bug reporting template](https://github.com/OpenCTI-Platform/xtm-composer/issues/new?template=bug_report.md). To suggest a new feature, please use the [feature request template](https://github.com/OpenCTI-Platform/xtm-composer/issues/new?template=feature_request.md).

* **Documentation improvements**: Don't hesitate to flag us an issue with the documentation if you find it incomplete or not clear enough. You can do that either by opening a [bug report](https://github.com/OpenCTI-Platform/xtm-composer/issues/new?template=bug_report.md) or by sending us a message on our [Slack channel](https://community.filigran.io).

* **Issue triage**: You can look through opened issues and help triage them (ask for more information, suggest workarounds, suggest labels, flag issues, etc.)

* **Code contributions**: If you are interested in contributing code to XTM Composer, please refer to our [development guide](docs/development.md). Whether fixing an issue that's meaningful to you or developing a feature requested by others, your contributions are welcome!


## Development Setup

For detailed information about setting up a development environment, please refer to our [Development Guide](docs/development.md).


## Commit Message Format

All commits messages must be formatted as:

```
[composer] Message (#issuenumber)
```

### Requirements

- **Component prefix**: All commits must start with `[composer]`
- **Signed commits**: All commits must be GPG signed (see [GitHub documentation on signed commits](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits))
- **Issue reference**: Include issue number when applicable using `(#issue)` format
- **Clear description**: Use clear, descriptive messages in present tense

### Examples

```
[composer] Add support for custom connector configurations (#123)
[composer] Fix Docker orchestration timeout issue (#456)
[composer] Update documentation for Kubernetes deployment (#789)
[composer] Refactor configuration validation logic (#234)
```

## How can you get in touch for other questions?

If you need support or wish to engage in a discussion about XTM Composer, feel free to:

- Join us on our [Slack channel](https://community.filigran.io)
- Send us an [email](mailto:contact@opencti.io)
- Open a [GitHub issue](https://github.com/OpenCTI-Platform/xtm-composer/issues) with the "question" label

We're always happy to help and discuss improvements to the project!
