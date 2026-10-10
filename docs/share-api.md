# The Dexcom Share protocol G7 Companion speaks

Both clients in this repository, the Python helper behind the Omarchy plugin
and the Rust tray app, implement the same three calls against Dexcom Share.
This file is the contract they are written to. If Dexcom changes something,
fix it here first, then in both clients, then in the tests.

Share is the service behind the Dexcom Follow app. It is not a published API.
It has been stable since 2015 and the Nightscout, xDrip, Sugarmate and Home
Assistant communities all run on it, which is why a change tends to be noticed
and documented within a day. Nothing here was obtained by decompiling Dexcom
software; it is the same request shape every one of those projects uses.

## Base URLs

| Region | Base |
|---|---|
| United States | `https://share2.dexcom.com/ShareWebServices/Services` |
| Outside the US | `https://shareous1.dexcom.com/ShareWebServices/Services` |
| Japan | `https://share.dexcom.jp/ShareWebServices/Services` |

Every call is an HTTPS `POST` with `Accept: application/json` and, when there
is a body, `Content-Type: application/json`. Redirects are never followed. A
response body is read with a hard byte ceiling, 4 KiB for the two login calls
and 256 KiB for readings.

The application ID is a constant that identifies "the Dexcom Share app" to the
service. It is the same for every user:

```
d89443d2-327c-4a6f-89e5-496bbb0317db
```

## 1. Authenticate: account name to account ID

```
POST {base}/General/AuthenticatePublisherAccount
{"accountName": "<username, email or +1 phone>", "password": "<password>", "applicationId": "<app id>"}
```

Returns a JSON string holding the account ID, a UUID. A UUID of all zeros
means the account was not found.

G7 accounts sign in with an email address or a phone number in international
form (`+15551234567`). A phone number works for the account that was created
with it. If sign-in fails with a correct password, the account ID itself can be
used instead: sign in at `uam1.dexcom.com` (US) or `uam2.dexcom.com` (outside
the US) and copy the UUID from the address bar. With an account ID in hand,
skip this step.

## 2. Log in: account ID to session ID

```
POST {base}/General/LoginPublisherAccountById
{"accountId": "<account id>", "password": "<password>", "applicationId": "<app id>"}
```

Returns a JSON string holding the session ID, also a UUID. A session lives for
a long time, often days. Cache it on disk and reuse it; only log in again when
a readings call says the session is gone.

## 3. Readings

```
POST {base}/Publisher/ReadPublisherLatestGlucoseValues?sessionId=<session>&minutes=<1..1440>&maxCount=<1..288>
```

Empty body. Returns a JSON array, newest first:

```json
[{"WT":"Date(1760112345000)","ST":"Date(1760112345000)","DT":"Date(1760112345000-0500)","Value":142,"Trend":"Flat"}]
```

- `WT` is the wall-clock time of the reading in milliseconds since the epoch,
  UTC. Use it. `ST` is the system time and `DT` is the display time with the
  phone's UTC offset.
- `Value` is mg/dL. Divide by 18.0 for mmol/L.
- `Trend` is a string on G7. Older firmware returned an integer. Both forms map
  as follows:

| Int | String | Arrow | Meaning |
|---|---|---|---|
| 0 | `None` | | no trend |
| 1 | `DoubleUp` | ↑↑ | rising fast |
| 2 | `SingleUp` | ↑ | rising |
| 3 | `FortyFiveUp` | ↗ | rising slowly |
| 4 | `Flat` | → | steady |
| 5 | `FortyFiveDown` | ↘ | falling slowly |
| 6 | `SingleDown` | ↓ | falling |
| 7 | `DoubleDown` | ↓↓ | falling fast |
| 8 | `NotComputable` | ? | trend not computable |
| 9 | `RateOutOfRange` | ? | rate out of range |

A G7 posts one reading every five minutes. Share lags the phone by a minute or
two. Polling more often than once a minute gains nothing.

## Errors

Failures come back as HTTP 500 with a JSON body:

```json
{"Code":"SessionNotValid","Message":"...","SubCode":"...","TypeName":"..."}
```

| Code | Means | Client does |
|---|---|---|
| `SessionIdNotFound`, `SessionNotValid` | the cached session is gone | drop it, log in once, retry the readings call once |
| `SSO_AuthenticateAccountNotFound` | no such account name | stop; the user has to sign in again |
| `SSO_AuthenticatePasswordInvalid`, `AccountPasswordInvalid` | wrong password | stop; the user has to sign in again |
| `SSO_AuthenticateMaxAttemptsExceeed` (Dexcom's spelling) | too many bad logins | back off for 30 minutes; do not retry sooner |
| `SSO_InternalError`, `InvalidArgument`, anything else | server side or malformed request | treat as a transient failure and keep the last reading |

The lockout is the one that bites. A client that polls every minute with a
stale password will lock the account within the hour, and the lock also hits
the phone that is actually wearing the sensor. So both clients here stop
authenticating entirely after a password failure until the user signs in
again, and never retry during a Dexcom back-off window.

## What the user has to supply

The credentials of the **sharer**: the Dexcom account the G7 app on the
patient's phone is signed into. A follower's own Dexcom account cannot read
anyone's data through these calls. Share must be turned on in the G7 app with
at least one follower, which is already true for anyone using Dexcom Follow.
Signing in here does not use one of the ten follower slots.
