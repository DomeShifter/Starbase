# Single sign-on (OpenID Connect)

If you already run a sign-in service for your home server (Authelia, Authentik, Keycloak,
Kanidm, Zitadel, Pocket ID…), StarSheets can use it: the sign-in box gets a "Sign in with SSO"
button. StarDocs uses the same sign-in, so it works there too. Usernames and passwords kept by
StarSheets keep working alongside, unless you turn that off.

## How accounts work

- The first time someone signs in through your service, StarSheets makes them an account
  (username from their name at the service, made unique; no password). Turn this off with
  `STARSHEETS_OIDC_AUTO_CREATE=off`: then the owner links people's accounts first.
- After that they always get the same account: it is found by the service's own id for them
  (issuer and subject), **never by email or username**, so nobody can take over an account by
  changing their email at the service. A StarSheets account called `ann` is not given to a
  service user called `ann`: they get `ann2`.
- Someone who already has a StarSheets account (the owner too) can link it: sign in with the
  password, then your avatar › Your name and colour › the SSO button.
- **Owners.** Nobody becomes owner just by signing in. With `STARSHEETS_OIDC_OWNER_GROUP`
  set, members of that group are owners and people who leave it stop being owners (checked at
  every sign-in; the last owner always stays one). On a new server you still make the owner's
  account with the setup code (see [deploy.md](deploy.md)), unless an owner group is set: then
  a member of it can set the server up by signing in.
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
| `STARSHEETS_OIDC_AUTO_CREATE` | `on` (default) or `off` |
| `STARSHEETS_OIDC_GROUPS_CLAIM` | Where the service lists someone's groups (default `groups`) |
| `STARSHEETS_OIDC_OWNER_GROUP` | Members are owners (optional) |
| `STARSHEETS_OIDC_ALLOWED_GROUP` | Only members (and owners) may sign in this way (optional) |
| `STARSHEETS_OIDC_ONLY` | `on`: no passwords and no sign-up, except owners' passwords (a way in when the service is down) |
| `STARSHEETS_OIDC_CLOCK_SKEW` | Seconds of clock difference allowed with the service (default 60) |

Things to know:

- Open StarSheets at the same address as the redirect URL: the sign-in is tied to the browser by
  a cookie for that address.
- Use HTTPS for anything reachable from outside your home.
- StarSheets checks the service's certificate against the computer's (or container's) own list.
  If your service uses a certificate from your own certificate authority, add it there (in
  Docker: mount it and point `SSL_CERT_FILE` at a file holding it and the usual ones).
- The server asks the service for its settings at the first sign-in and once an hour after; if
  sign-in fails, the reason is in the server's log.

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

## Others

Any OpenID Connect service with discovery (`/.well-known/openid-configuration`), the
authorization code flow and PKCE works. Kanidm: the issuer is per client
(`https://idm.example.com/oauth2/openid/starsheets`). Zitadel: groups are roles in an object;
set `STARSHEETS_OIDC_GROUPS_CLAIM` to its roles claim and its role names become the groups.
