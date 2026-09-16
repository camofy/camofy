# Subscription refresh retries

Each manual, scheduled, initial or settings-triggered refresh is one logical job
and one history row. It attempts at most three times, each limited to 30 seconds,
within the existing 100-second total deadline and 120-second database lease.
Delays after the first and second failures are 2–3 and 4–5 seconds. Jitter avoids
synchronized bursts. No new scheduler, service or deployment setting is required.

Retryable failures: timeouts, connection/transport interruptions, and HTTP
408/429/500/502/503/504. Credential/permission errors, explicit provider business
errors (including missing whitelist/credit), unsafe destinations, malformed
responses and invalid YAML are not blindly retried. Provider network errors keep
only a sanitized retry classification, never their secret URLs or response bodies.
`Retry-After` delta seconds is respected; a delay beyond the remaining deadline or
an unrecognized/date-form value ends this refresh rather than retrying too soon.

Every attempt runs platform proxy admission, extraction and subscription fetch again.
Short lived proxy addresses are not reused. A missing platform proxy fails closed;
no attempt falls back to direct. Existing global supplier rate limiting applies to each attempt.
The same lease remains held during backoff, excluding other workers. Before the
next attempt, changes to request ID, configuration, proxy version or lease ownership
stop the old retry sequence; the existing publication/version checks still apply.
Platform policy changes also cancel in-flight attempts/backoff via the existing
one-second watcher; publication retains the shared policy lock.

History displays sanitized progress, final attempt count and prior failure codes
through its existing message field and five-second UI refresh. Success publishes
once; exhausted failure preserves the last successful YAML and marks usage stale.
Manual-only jobs stop after completion; scheduled jobs return to their normal
interval. This is bounded recovery from intermittent failures, not a fix for invalid
credentials or an unavailable supplier. Worst-case extraction usage is three calls
per refresh, not one.

Verification:

```sh
cargo test --locked --lib --bin camofy-cloud
# Use only a disposable local database, never production:
TEST_DATABASE_URL=postgres://... cargo test --locked --bin camofy-cloud subscription_retry_end_to_end -- --ignored
```

Integration uses a local fake proxy and tests eventual success, exhaustion,
permanent HTTP errors, competing workers, preservation of cached configuration,
manual requests during backoff and absence of secrets in history.
