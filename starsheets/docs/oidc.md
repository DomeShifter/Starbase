# Single sign-on (OpenID Connect)

If you already run a sign-in service for your home server (Authelia, Authentik, Keycloak,
Kanidm, Zitadel, Pocket ID…), Starbase Sheets can use it: the sign-in box gets a "Sign in with SSO"
button. Starbase Docs uses the same sign-in, so it works there too. Usernames and passwords kept by
Starbase Sheets keep working alongside, unless you turn that off.

## How accounts work

- **New accounts.** Someone signing in through your service for the first time gets an account
  (username from their name at the service, made unique; no password) only when:
  `STARBASE_OIDC_AUTO_CREATE=on`; or, with that unset, `STARBASE_OIDC_ALLOWED_GROUP` is set
  (your service decides who), or the owner has turned on sign-up (People › "Let people make
  their own account") with no sign-up code. Otherwise they are told there is no account for
  them: the owner adds them (or turns sign-up on), and they link it. With `AUTO_CREATE=on` and
  no allowed group, everyone who can sign in at your service gets an account, and the server's
  log warns about that at start.
- After that they always get the same account: it is found by the service's own id for them
  (issuer and subject), **never by email or username**, so nobody can take over an account by
  changing their email at the service. A Starbase Sheets account called `ann` is not given to a
  service user called `ann`: they get `ann2`.
- **Linking.** Someone who already has a Starbase Sheets account (the owner too) can link it: sign
  in with the password, then your avatar › Your name and colour, type your password and press
  the SSO button. The service then asks who to link (`prompt=login`), even when someone is
  already signed in there.
- **An account made through your service** has no password, so it can't simply be unlinked
  (it couldn't sign in at all then). In Your name and colour it has "Confirm who you are": the
  service asks them to sign in again (`api/oidc/login?fresh=1`: `prompt=login`, `max_age=0`),
  and for the next 10 minutes they can **Set a password** (after which Unlink works, and the
  password signs in too) or **Link another sign-in**. Renewing a session without asking never
  counts as confirming.
- A new password removes the link (whoever links an account must know its current password),
  unless it was changed while signed in through your service. A password the owner sets in
  People removes it too. The owner sees in People whom each account is linked to ("ann@example.com
  at Keycloak": what the service called them at their last sign-in, and the service's name,
  `STARBASE_OIDC_LABEL` or its address; the service's own ids show on hover), with "Remove
  link" (not for an account with no password). Removing a link ends the sessions made through it
  at once. With `STARBASE_OIDC_ONLY` only owners can unlink.
- **Owners.** Nobody becomes owner just by signing in. With `STARBASE_OIDC_OWNER_GROUP`
  set, members of that group are owners (they still need an account: see New accounts) and
  people who leave it stop being owners (checked at
  every sign-in; their other sessions end then; the last owner always stays one). The group
  only takes away what it gave: an owner made with the setup code stays owner whatever group
  the sign-in they link is in, so linking can't lock them out. On a new server you still make
  the owner's account with the setup code (see [deploy.md](deploy.md)), unless an owner group
  is set: then a member of it can set the server up by signing in.
- **Sessions.** Signing in through your service lasts 12 hours
  (`STARBASE_OIDC_SESSION_HOURS`), not 30 days like a password sign-in. Then the app goes
  back to your service without asking anything (`prompt=none`): someone still signed in there
  carries on; someone removed there, or out of the allowed group, gets the sign-in box. A page
  left open does this too: about 10 minutes before the end (or when a request finds the
  session ended), at a quiet moment (nothing waiting to be saved, no cell being typed in, no
  dialog open, a few seconds without input), and comes back to the same file. If changes can't
  be saved for a minute because the session has ended, the sign-in box shows instead. Your
  service must not stop at a consent page each time (see Authelia below). So a change at your
  service reaches Starbase Sheets within that time, and at once for anyone who signs in again (taken
  out of the allowed group: their sessions through your service end; out of the owner group:
  all their other sessions end).
- Signing out also signs out at the service when it offers that (Keycloak, Authentik; Authelia
  doesn't, so only Starbase Sheets signs out there).

## Settings

The older names (`STARSHEETS_OIDC_…`) still work; when both are set the `STARBASE_` one wins.

| Setting | What it does |
|---|---|
| `STARBASE_OIDC_ISSUER` | Your service's issuer address, exactly as it gives it (Authentik's ends in `/`). Setting it turns single sign-on on |
| `STARBASE_OIDC_CLIENT_ID` | The client id you registered for Starbase Sheets |
| `STARBASE_OIDC_CLIENT_SECRET` or `STARBASE_OIDC_CLIENT_SECRET_FILE` | Its secret (or a file holding it, e.g. a Docker secret) |
| `STARBASE_OIDC_REDIRECT_URL` | The address people open Starbase Sheets at, then `api/oidc/callback`, e.g. `https://sheets.example.com/api/oidc/callback`. Register the same address at the service |
| `STARBASE_OIDC_SCOPES` | Default `openid profile email groups` |
| `STARBASE_OIDC_BUTTON_LABEL` | Default `Sign in with SSO` |
| `STARBASE_OIDC_LABEL` | Your service's name as people know it (e.g. `Keycloak`), shown in People. Default: the issuer's host |
| `STARBASE_OIDC_AUTO_CREATE` | `on` or `off`. Unset: on with an allowed group, else only while sign-up is on with no code |
| `STARBASE_OIDC_SESSION_HOURS` | How long a sign-in through your service lasts before it is renewed there (default 12, 1 to 720) |
| `STARBASE_OIDC_GROUPS_CLAIM` | Where the service lists someone's groups (default `groups`) |
| `STARBASE_OIDC_OWNER_GROUP` | Members are owners (optional) |
| `STARBASE_OIDC_ALLOWED_GROUP` | Only members (and owners) may sign in this way (optional) |
| `STARBASE_OIDC_ONLY` | `on`: no passwords and no sign-up, except owners' passwords (a way in when the service is down). Anyone else's password gets the same "isn't right" answer as a wrong one |
| `STARBASE_OIDC_CLOCK_SKEW` | Seconds of clock difference allowed with the service (default 60) |

Things to know:

- Open Starbase Sheets at the same address as the redirect URL: the sign-in is tied to the browser by
  a cookie for that address.
- Both addresses must be `https://` (Starbase Sheets refuses to start otherwise). Plain `http://`
  works only for `localhost`, `127.0.0.1` and `::1`, for trying things out.
- Over HTTPS the cookies are called `__Host-starsheets_session` and `__Host-starsheets_oidc`,
  which another site on a sibling or parent address (`other.example.com`, `example.com`) can't
  set. The plain names aren't read over HTTPS at all, so after updating from 0.4 people sign in
  once more. Behind a proxy, Starbase Sheets knows a request came over HTTPS only from
  `STARBASE_TRUSTED_PROXY` (or `STARBASE_SECURE_COOKIE=on`): set one of them.
- If you tried a pre-release build of single sign-on (never in a release): owners the owner
  group made then stay owners after leaving the group (fix by hand: `"owner": false` for them
  in `.starsheets/users.json`, with the server stopped), and sessions made then last 30 days
  (deleting `.starsheets/sessions.json` signs everyone out).
- Starbase Sheets checks the service's certificate against the computer's (or container's) own list:
  see "Your own certificate" below.
- The server reads the service's settings at start (a problem shows in its log straight away),
  again when a sign-in starts if they are more than 30 seconds old (so a service that is down
  gives Starbase Sheets' own "couldn't be reached" message), and once an hour otherwise.
- If sign-in fails, the reason is in the server's log. A wrong client secret, for example, only
  shows after the person has signed in at the service, as "didn't work".
- The Starbase Sheets server must be able to reach your service by its name (real DNS, or
  `--add-host` in Docker).

## Your own certificate

If your service has a certificate you made yourself, Starbase Sheets must trust it, and it must not
be marked as a certificate authority. The usual one-liner `openssl req -x509 …` makes one that
*is* marked so (`CA:TRUE`): Starbase Sheets refuses it even when trusted (the log says
`CaUsedAsEndEntity`), while browsers and curl accept it. Any of these works:

- Authelia's own command (the easiest; it writes `public.crt` and `private.pem`, so use
  `public.crt` where `auth.crt` is below):
  `authelia crypto certificate rsa generate --common-name auth.example.com --sans auth.example.com`
- openssl with `-addext basicConstraints=critical,CA:FALSE` added, for example
  `openssl req -x509 -newkey rsa:2048 -nodes -days 825 -keyout auth.key -out auth.crt -subj /CN=auth.example.com -addext subjectAltName=DNS:auth.example.com -addext basicConstraints=critical,CA:FALSE`
- your own certificate authority, with a server certificate it signed (the service sends both).

Then trust it: make one file with the usual list and yours,
`cat /etc/ssl/certs/ca-certificates.crt auth.crt > bundle.crt` (your CA's certificate instead
of `auth.crt` with your own authority), mount it into the container and set
`SSL_CERT_FILE=/path/to/bundle.crt`. Without it the log says `UnknownIssuer`.

## Authelia

In Authelia's `configuration.yml` (make the hashed secret with
`authelia crypto hash generate pbkdf2 --random --random.length 64`):

```yaml
identity_providers:
  oidc:
    clients:
      - client_id: starsheets
        client_name: Starbase Sheets
        client_secret: '$pbkdf2-sha512$...'   # the hash; Starbase Sheets gets the plain secret
        public: false
        authorization_policy: two_factor
        require_pkce: true
        pkce_challenge_method: S256
        redirect_uris:
          - https://sheets.example.com/api/oidc/callback
        scopes: [openid, profile, email, groups]
        response_types: [code]
        grant_types: [authorization_code]
        token_endpoint_auth_method: client_secret_basic
        consent_mode: implicit
```

`consent_mode: implicit` matters: without it Authelia shows its consent page ("Consent Request:
ACCEPT / DENY") at the first sign-in and again every time Starbase Sheets renews a session
(`STARBASE_OIDC_SESSION_HOURS`), even for a renewal that asks for no page (`prompt=none`).
Then renewing stops on that page instead of carrying on unnoticed. If you'd rather people agree
once, use `consent_mode: pre-configured` with `pre_configured_consent_duration` (for example
`1y`).

```
STARBASE_OIDC_ISSUER=https://auth.example.com
STARBASE_OIDC_CLIENT_ID=starsheets
STARBASE_OIDC_CLIENT_SECRET_FILE=/run/secrets/starsheets_oidc
STARBASE_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARBASE_OIDC_OWNER_GROUP=admins
```

Authelia gives groups only through userinfo, which Starbase Sheets always reads.
`authorization_policy: two_factor` asks for a second factor; to try things out without one set
up, use `one_factor`.

Authelia keeps each person's id for Starbase Sheets (the subject) in its own database. If that
database is reset (or `storage.encryption_key` is lost), everyone gets a new id, and their next
sign-in makes a second account (`alice2`). So keep Authelia's database. If it happens anyway:
the owner removes the new accounts (People › Remove; their files go to the owner), then each
person signs in to their old account with its password and links it again (an account made
through single sign-on has no password: the owner sets one first in People).

## Authentik

Applications › Providers › Create › **OAuth2/OpenID Provider**: client type *Confidential*,
redirect URI `https://sheets.example.com/api/oidc/callback` (strict), a signing key chosen
(RS256), scopes `openid`, `email`, `profile`, and as Authorization flow
`default-provider-authorization-implicit-consent` (the explicit one shows a consent page, which
renewing a session can't get past). Then an Application using it, slug `starsheets`.
Authentik sends groups in the `profile` scope as `groups`.

```
STARBASE_OIDC_ISSUER=https://authentik.example.com/application/o/starsheets/
STARBASE_OIDC_CLIENT_ID=<Client ID from the provider>
STARBASE_OIDC_CLIENT_SECRET=<Client Secret from the provider>
STARBASE_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARBASE_OIDC_SCOPES=openid profile email
STARBASE_OIDC_OWNER_GROUP=authentik Admins
```

Note the `/` at the end of the issuer: Authentik's issuer has it, and it must match exactly.

## Keycloak

In your realm: Clients › Create client › OpenID Connect, client id `starsheets`; *Client
authentication* on, *Standard flow* only; Valid redirect URIs
`https://sheets.example.com/api/oidc/callback`; Valid post logout redirect URIs
`https://sheets.example.com/`; Advanced › PKCE method `S256`; leave *Consent required* (Login
settings) off, its default, or renewing a session stops at a consent page. Groups: Client scopes › Create
`groups` with a **Group Membership** mapper (token claim name `groups`, added to the ID token
and userinfo; "Full group path" on or off, both work), then add it to the client as a Default
scope.

```
STARBASE_OIDC_ISSUER=https://keycloak.example.com/realms/home
STARBASE_OIDC_CLIENT_ID=starsheets
STARBASE_OIDC_CLIENT_SECRET=<Credentials tab>
STARBASE_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARBASE_OIDC_SCOPES=openid profile email
STARBASE_OIDC_OWNER_GROUP=admins
```

`groups` isn't in the scopes on purpose: the `groups` client scope above is a *default* scope,
so Keycloak adds the groups claim (through its Group Membership mapper) without being asked.
Keycloak also asks people with no first name, last name or email for them at their first
sign-in (that is Keycloak, not Starbase Sheets).

## Others

Any OpenID Connect service with discovery (`/.well-known/openid-configuration`), the
authorization code flow and PKCE works. Kanidm: the issuer is per client
(`https://idm.example.com/oauth2/openid/starsheets`). Zitadel: groups are roles in an object;
set `STARBASE_OIDC_GROUPS_CLAIM` to its roles claim and its role names become the groups.
