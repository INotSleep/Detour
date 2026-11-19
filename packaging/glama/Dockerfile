# Detour's MCP server for Glama's checks (glama.ai/mcp/servers/INotSleep/Detour): the
# command line from the release, started as `detour mcp`. It answers initialize and
# tools/list without a vault; real use needs Detour where the person approves access.
FROM debian:bookworm-slim
ARG VERSION=0.5.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && curl -fsSL "https://github.com/INotSleep/Detour/releases/download/v${VERSION}/detour-cli-${VERSION}-linux-x86_64.tar.gz" | tar xz -C /usr/local/bin detour \
 && apt-get purge -y curl && apt-get autoremove -y && rm -rf /var/lib/apt/lists/*
ENV DETOUR_NO_TRAY=1
ENTRYPOINT ["detour", "mcp"]
