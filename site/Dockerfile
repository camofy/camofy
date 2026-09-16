FROM caddy:2-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648
RUN setcap -r /usr/bin/caddy
COPY Caddyfile /etc/caddy/Caddyfile
COPY index.html robots.txt sitemap.xml /srv/
COPY site-assets/ /srv/site-assets/
USER 10001:10001
ENV XDG_CONFIG_HOME=/tmp/config XDG_DATA_HOME=/tmp/data
EXPOSE 8080
