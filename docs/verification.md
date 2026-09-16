# Verification guide

Use isolated databases, mock cores and synthetic accounts for automated checks.

- Run Rust formatting, unit tests and Clippy for the relevant features.
- Run the PostgreSQL integration suites with an explicit test database URL.
- Build the mock core and Agent for rollback and offline-restoration tests.
- Build/lint the web application and inspect desktop/mobile layouts and affected controls.
- Validate authorization boundaries, credential handling, failed updates and recovery.

CI defines the executable check sequence. Hardware-specific acceptance requires
separate testing with the device owner's authorization; a passing mock-core test
is not proof of gateway traffic correctness. Keep private deployment evidence,
network addresses and real account data out of commits and fixtures.
