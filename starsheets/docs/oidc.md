# Single sign-on (OpenID Connect)

If you already run a sign-in service for your home server (Authelia, Authentik, Keycloak,
Kanidm, Zitadel, Pocket ID…), StarSheets can use it: the sign-in box gets a "Sign in with SSO"
button. StarDocs uses the same sign-in, so it works there too. Usernames and passwords kept by
StarSheets keep working alongside, unless you turn that off.

## How accounts work

- **New accounts.** Someone signing in through your service for the first time gets an account
  (username from their name at the service, made unique; no password) only when:
  `STARSHEETS_OIDC_AUTO_CREATE=on`; or, with that unset, `STARSHEETS_OIDC_ALLOWED_GROUP` is set
  (your service decides who), or the owner has turned on sign-up (People › "Let people make
  their own account") with no sign-up code. Otherwise they are told there is no account for
  them: the owner adds them (or turns sign-up on), and they link it. With `AUTO_CREATE=on` and
  no allowed group, everyone who can sign in at your service gets an account, and the server's
  log warns about that at start.
- After that they always get the same account: it is found by the service's own id for them
  (issuer and subject), **never by email or username**, so nobody can take over an account by
  changing their email at the service. A StarSheets account called `ann` is not given to a
  service user called `ann`: they get `ann2`.
- **Linking.** Someone who already has a StarSheets account (the owner too) can link it: sign
  in with the password, then your avatar › Your name and colour, type your password and press
  the SSO button. An account with no password (made through your service) can link a different
  sign-in only within 10 minutes of a sign-in where the service asked who they are again
  (`api/oidc/login?fresh=1`, which asks for `prompt=login` and `max_age=0`; renewing a session
  without asking never counts).
- A new password removes the link (whoever links an account must know its current password),
  unless it was changed while signed in through your service. A password the owner sets in
  People removes it too. The owner sees in People whom each account is linked to, with
  "Remove link". Removing a link ends the sessions made through it at once.
- **Owners.** Nobody becomes owner just by signing in. With `STARSHEETS_OIDC_OWNER_GROUP`
  set, members of that group are owners and people who leave it stop being owners (checked at
  every sign-in; their other sessions end then; the last owner always stays one). The group
  only takes away what it gave: an owner made with the setup code stays owner whatever group
  the sign-in they link is in, so linking can't lock them out. On a new server you still make
  the owner's account with the setup code (see [deploy.md](deploy.md)), unless an owner group
  is set: then a member of it can set the server up by signing in.
- **Sessions.** Signing in through your service lasts 12 hours
  (`STARSHEETS_OIDC_SESSION_HOURS`), not 30 days like a password sign-in. Then the app goes
  back to your service without asking anything: someone still signed in there carries on;
  someone removed there, or out of the allowed group, gets the sign-in box. So a change at your
  service reaches StarSheets within that time, and at once for anyone who signs in again (taken
  out of the allowed group: their sessions through your service end; out of the owner group:
  all their other sessions end).
- Signing out also signs out at the service when it offers that (Keycloak, Authentik; Authelia
  doesn't, so only StarSheets signs out there).

## Settings

| Setting | What it does |
|---|---|
| `STARSHEETS_OIDC_ISSUER` | Your service's issuer address, exactly as it gives it (Authentik's ends in `/`). Setting it turns single sign-on on |
| `STARSHEETS_OIDC_CLIENT_ID` | The client id you registered for StarSheets |
| `STARSHEETS_OIDC_CLIENT_SECRET` or `STARSHEETS_OIDC_CLIENT_SECRET_FILE` | Its secret (or a file holding it, e.g. a Docker secret) |
| `STARSHEETS_OIDC_REDIRECT_URL` | The address people open StarSheets at, then `api/oidc/callback`, e.g. `https://sheets.example.com/api/oidc/callback`. Register the same address at the service |
| `STARSHEETS_OIDC_SCOPES` | Default `openid profile email groups` |
| `STARSHEETS_OIDC_BUTTON_LABEL` | Default `Sign in with SSO` |
| `STARSHEETS_OIDC_AUTO_CREATE` | `on` or `off`. Unset: on with an allowed group, else only while sign-up is on with no code |
| `STARSHEETS_OIDC_SESSION_HOURS` | How long a sign-in through your service lasts before it is renewed there (default 12, 1 to 720) |
| `STARSHEETS_OIDC_GROUPS_CLAIM` | Where the service lists someone's groups (default `groups`) |
| `STARSHEETS_OIDC_OWNER_GROUP` | Members are owners (optional) |
| `STARSHEETS_OIDC_ALLOWED_GROUP` | Only members (and owners) may sign in this way (optional) |
| `STARSHEETS_OIDC_ONLY` | `on`: no passwords and no sign-up, except owners' passwords (a way in when the service is down). Anyone else's password gets the same "isn't right" answer as a wrong one |
| `STARSHEETS_OIDC_CLOCK_SKEW` | Seconds of clock difference allowed with the service (default 60) |

Things to know:

- Open StarSheets at the same address as the redirect URL: the sign-in is tied to the browser by
  a cookie for that address.
- Both addresses must be `https://` (StarSheets refuses to start otherwise). Plain `http://`
  works only for `localhost`, `127.0.0.1` and `::1`, for trying things out.
- Over HTTPS the cookies are called `__Host-starsheets_session` and `__Host-starsheets_oidc`,
  which another site on a sibling or parent address (`other.example.com`, `example.com`) can't
  set. The plain names aren't read over HTTPS at all, so after updating from 0.4 people sign in
  once more. Behind a proxy, StarSheets knows a request came over HTTPS only from
  `STARSHEETS_TRUSTED_PROXY` (or `STARSHEETS_SECURE_COOKIE=on`): set one of them.
- If you tried a pre-release build of single sign-on (never in a release): owners the owner
  group made then stay owners after leaving the group (fix by hand: `"owner": false` for them
  in `.starsheets/users.json`, with the server stopped), and sessions made then last 30 days
  (deleting `.starsheets/sessions.json` signs everyone out).
- StarSheets checks the service's certificate against the computer's (or container's) own list:
  see "Your own certificate" below.
- The server reads the service's settings at start (a problem shows in its log straight away),
  again when a sign-in starts if they are more than 30 seconds old (so a service that is down
  gives StarSheets' own "couldn't be reached" message), and once an hour otherwise.
- If sign-in fails, the reason is in the server's log. A wrong client secret, for example, only
  shows after the person has signed in at the service, as "didn't work".
- The StarSheets server must be able to reach your service by its name (real DNS, or
  `--add-host` in Docker).

## Your own certificate

If your service has a certificate you made yourself, StarSheets must trust it, and it must not
be marked as a certificate authority. The usual one-liner `openssl req -x509 …` makes one that
*is* marked so (`CA:TRUE`): StarSheets refuses it even when trusted (the log says
`CaUsedAsEndEntity`), while browsers and curl accept it. Any of these works:

- Authelia's own command (the easiest):
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
        client_name: StarSheets
        client_secret: '$pbkdf2-sha512$...'   # the hash; StarSheets gets the plain secret
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
```

```
STARSHEETS_OIDC_ISSUER=https://auth.example.com
STARSHEETS_OIDC_CLIENT_ID=starsheets
STARSHEETS_OIDC_CLIENT_SECRET_FILE=/run/secrets/starsheets_oidc
STARSHEETS_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARSHEETS_OIDC_OWNER_GROUP=admins
```

Authelia gives groups only through userinfo, which StarSheets always reads.
`authorization_policy: two_factor` asks for a second factor; to try things out without one set
up, use `one_factor`.

Authelia keeps each person's id for StarSheets (the subject) in its own database. If that
database is reset (or `storage.encryption_key` is lost), everyone gets a new id, and their next
sign-in makes a second account (`alice2`). So keep Authelia's database. If it happens anyway:
the owner removes the new accounts (People › Remove; their files go to the owner), then each
person signs in to their old account with its password and links it again (an account made
through single sign-on has no password: the owner sets one first in People).

## Authentik

Applications › Providers › Create › **OAuth2/OpenID Provider**: client type *Confidential*,
redirect URI `https://sheets.example.com/api/oidc/callback` (strict), a signing key chosen
(RS256), scopes `openid`, `email`, `profile`. Then an Application using it, slug `starsheets`.
Authentik sends groups in the `profile` scope as `groups`.

```
STARSHEETS_OIDC_ISSUER=https://authentik.example.com/application/o/starsheets/
STARSHEETS_OIDC_CLIENT_ID=<Client ID from the provider>
STARSHEETS_OIDC_CLIENT_SECRET=<Client Secret from the provider>
STARSHEETS_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARSHEETS_OIDC_SCOPES=openid profile email
STARSHEETS_OIDC_OWNER_GROUP=authentik Admins
```

Note the `/` at the end of the issuer: Authentik's issuer has it, and it must match exactly.

## Keycloak

In your realm: Clients › Create client › OpenID Connect, client id `starsheets`; *Client
authentication* on, *Standard flow* only; Valid redirect URIs
`https://sheets.example.com/api/oidc/callback`; Valid post logout redirect URIs
`https://sheets.example.com/`; Advanced › PKCE method `S256`. Groups: Client scopes › Create
`groups` with a **Group Membership** mapper (token claim name `groups`, added to the ID token
and userinfo; "Full group path" on or off, both work), then add it to the client as a Default
scope.

```
STARSHEETS_OIDC_ISSUER=https://keycloak.example.com/realms/home
STARSHEETS_OIDC_CLIENT_ID=starsheets
STARSHEETS_OIDC_CLIENT_SECRET=<Credentials tab>
STARSHEETS_OIDC_REDIRECT_URL=https://sheets.example.com/api/oidc/callback
STARSHEETS_OIDC_SCOPES=openid profile email
STARSHEETS_OIDC_OWNER_GROUP=admins
```

`groups` isn't in the scopes on purpose: the `groups` client scope above is a *default* scope,
so Keycloak adds the groups claim (through its Group Membership mapper) without being asked.
Keycloak also asks people with no first name, last name or email for them at their first
sign-in (that is Keycloak, not StarSheets).

## Others

Any OpenID Connect service with discovery (`/.well-known/openid-configuration`), the
authorization code flow and PKCE works. Kanidm: the issuer is per client
(`https://idm.example.com/oauth2/openid/starsheets`). Zitadel: groups are roles in an object;
set `STARSHEETS_OIDC_GROUPS_CLAIM` to its roles claim and its role names become the groups.
