# API example contributor guide

Inspect local Git state and preserve existing work before changes or remote comparisons. Keep this repository separate from its surrounding local workspace. Use existing SSH authentication for authorized publication; never expose credentials or replace local source automatically from a remote.

Public documentation should help developers install the examples, authenticate, submit requests, handle attachments, poll results, and continue or retry safely. Explain supported API shapes and limits accurately. Keep backend architecture, operational investigations, and internal acceptance records out of user guides and release notes. Preserve required licenses and attribution.

Keep passwords, tokens, private data, environments, runtime output, and local operational notes out of Git. Use synthetic credentials and local mock services for ordinary validation. Do not contact live services merely to test documentation.

Run the Python tests and syntax checks for documentation/example updates. For Rust source changes, also run the documented formatting, test, and lint commands using a compatible toolchain. Record unavailable checks honestly. Review changed files and links before committing; publish only within the user's authorized scope.
