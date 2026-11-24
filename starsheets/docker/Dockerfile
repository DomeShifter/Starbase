# StarSheets in one container: the web app and its server (crates/server).
# Built by CI on each release: the site comes from the release's web build (./site) and the server
# program from the server build (./server-bin/<arch>/starsheets-server).
#   docker run -p 8080:8080 -v starsheets:/data ghcr.io/domeshifter/starsheets
FROM debian:13.7-slim
ARG TARGETARCH
RUN useradd -u 1000 -M -s /usr/sbin/nologin starsheets && mkdir -p /data && chown starsheets /data
COPY --chmod=755 server-bin/${TARGETARCH}/starsheets-server /usr/local/bin/starsheets-server
COPY site/ /site/
ENV STORE=/data SITE=/site PORT=8080
EXPOSE 8080
VOLUME /data
USER starsheets
HEALTHCHECK --interval=60s --timeout=5s CMD ["starsheets-server", "healthcheck"]
CMD ["starsheets-server"]
