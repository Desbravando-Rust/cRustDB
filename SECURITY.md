# Security Policy

Security is a priority for this project, even in its early development stages. Below are the guidelines regarding the current support status and how to responsibly report security vulnerabilities.

## Supported Versions

Currently, the project is in the pre-release phase (`0.x.x` versions) and **does not yet have a stable release recommended for production use.** 

Because this is an active development environment, security updates and bug fixes will be applied to the latest version through our standard Pull Request review process. There will be no backports for previous alpha/beta versions.

| Version | Supported | Notes |
| :--- | :--- | :--- |
| **0.1.x** | ✅ Yes | Active development. Always update to the latest version. |

## Reporting a Vulnerability

We greatly appreciate the community and testers who help us identify security flaws before our official release (v1.0.0).

To report a vulnerability or an architectural design flaw that could compromise security, **please open an issue on our GitHub repository**. 

When creating the issue, please provide the following information:
1. A clear and detailed description of the vulnerability and its potential impact.
2. Exact steps to reproduce the issue, along with the testing environment used.
3. Proof of Concept (PoC) or test scripts, if available.

If you consider the vulnerability highly critical and prefer not to disclose it publicly right away, you can also use GitHub's **Private vulnerability reporting** tool (located in the repository's "Security" tab, if enabled).

## Triage and Resolution Process

Since the project is in early development (v0.1.0), reported security fixes will be prioritized. A patch will be developed in a separate branch and go through our standard code review and Pull Request workflow before being merged into the main codebase.

We will gladly credit your contribution publicly once the fix is safely integrated and released.