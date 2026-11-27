# Starbase in one container: Starbase Sheets (with the home screen) at /, Starbase Docs at /docs/,
# Starbase Slides at /slides/, and the file server (starsheets/crates/server) that stores everything
# and serves the sites.
# Assembled by .github/workflows/release.yml; the build context holds
#   server-bin/<arch>/starsheets-server   (Starbase Sheets workflow, artifact starsheets-server)
#   site/                                  (artifact starsheets-web)
#   site/docs/                             (artifact stardocs-web)
#   site/slides/                           (artifact slides-web)
#   docker run -d -p 8080:8080 -v starbase:/data ghcr.io/domeshifter/starbase
FROM debian:13.7-slim
ARG TARGETARCH
# The certificate list: single sign-on (starsheets/docs/oidc.md) checks the sign-in service's HTTPS against it.
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates libstdc++6 ffmpeg && rm -rf /var/lib/apt/lists/*
RUN useradd -u 1000 -M -s /usr/sbin/nologin starbase && mkdir -p /data /backups && chown starbase /data /backups
COPY --chmod=755 server-bin/${TARGETARCH}/starsheets-server /usr/local/bin/starsheets-server
COPY site/ /site/
ENV STORE=/data SITE=/site PORT=8080 LOCATION="the server"
EXPOSE 8080
VOLUME /data
USER starbase
HEALTHCHECK --interval=60s --timeout=5s CMD ["starsheets-server", "healthcheck"]
CMD ["starsheets-server"]
