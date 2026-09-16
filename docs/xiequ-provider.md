# Short-lived fetch proxies

Configure the supported provider in the authenticated fetch-proxy form. Keep API
URLs, account identifiers and whitelist credentials in private configuration.
Preview and confirm the server egress before authorizing it. Existing whitelist
entries must be preserved.

Each refresh obtains a new endpoint and uses the selected protocol. A failed fetch
retains the last valid subscription; it does not fall back to direct access.
Provider responses and URLs containing credentials must not enter public logs.

Verify with disposable accounts and isolated fixtures. Store real endpoints,
account names, deployment receipts and screenshots outside this repository.
