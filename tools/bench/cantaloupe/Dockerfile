# Cantaloupe eval image for docs/bench/cantaloupe-eval.md: the strongest
# reproducible open-source configuration we could stand up. Build:
#   docker build -t cantaloupe-eval:openjpeg tools/bench/cantaloupe
# Run via scripts/bench_cantaloupe.sh.
#
# Built from the develop branch at a pinned commit — the 5.0.7 release
# bundles 2021-era libraries with dozens of known CVEs; develop carries
# the finished modernization unreleased. The dependency bumps below are
# security backports offered upstream as cantaloupe-project/cantaloupe#962.
# TARBALL_SHA256 is deliberately manual (fail-closed on SHA bumps).
# libopenjp2-tools supplies the JP2 processor: the only one an open-source
# deployment gets from a package manager.
#
FROM busybox:stable@sha256:73aaf090f3d85aa34ee199857f03fa3a95c8ede2ffd4cc2cdb5b94e566b11662 AS fetch

SHELL ["/bin/ash", "-o", "pipefail", "-c"]

# renovate: datasource=git-refs depName=cantaloupe-develop packageName=https://github.com/cantaloupe-project/cantaloupe
ARG CANTALOUPE_SHA=377942bd274b01ffdb106b08913d8aa5da7233de
ARG TARBALL_SHA256=8936076783c99bc1e3326466db981c7344fea2f77c55fbca0ec86749d254c7d1

RUN wget -q -O /tmp/src.tar.gz \
    "https://github.com/cantaloupe-project/cantaloupe/archive/${CANTALOUPE_SHA}.tar.gz" \
    && echo "${TARBALL_SHA256}  /tmp/src.tar.gz" | sha256sum -c - \
    && mkdir /src \
    && tar xzf /tmp/src.tar.gz -C /src --strip-components=1

FROM maven:3-eclipse-temurin-21@sha256:2b4496088e7b80ae10a8c9f74e574ea21380325a006ec684532ad6bad5bc7273 AS build

# Security backports upstream hasn't merged yet (see #962). bouncycastle is
# shaded inside jruby-complete — unreachable by dependencyManagement — so its
# CVE fixes arrive via the jruby bump.
# renovate: datasource=maven depName=ch.qos.logback:logback-classic
ARG LOGBACK_VERSION=1.5.34
# renovate: datasource=maven depName=org.eclipse.jetty:jetty-server
ARG JETTY_VERSION=12.1.10
# renovate: datasource=maven depName=org.jruby:jruby
ARG JRUBY_VERSION=10.1.1.0
# renovate: datasource=maven depName=io.netty:netty-bom
ARG NETTY_VERSION=4.1.136.Final
# renovate: datasource=maven depName=org.apache.commons:commons-compress
ARG COMMONS_COMPRESS_VERSION=1.28.0
# renovate: datasource=maven depName=org.codehaus.plexus:plexus-utils
ARG PLEXUS_UTILS_VERSION=4.0.3
# renovate: datasource=maven depName=org.iq80.snappy:snappy
ARG SNAPPY_VERSION=0.5

COPY --from=fetch /src /src
WORKDIR /src
RUN sed -i \
    -e "s|<logback.version>[^<]*</logback.version>|<logback.version>${LOGBACK_VERSION}</logback.version>|" \
    -e "s|<jetty.version>[^<]*</jetty.version>|<jetty.version>${JETTY_VERSION}</jetty.version>|" \
    -e "s|<jruby.version>[^<]*</jruby.version>|<jruby.version>${JRUBY_VERSION}</jruby.version>|" \
    pom.xml \
    && sed -i "s|</properties>|</properties><dependencyManagement><dependencies><dependency><groupId>io.netty</groupId><artifactId>netty-bom</artifactId><version>${NETTY_VERSION}</version><type>pom</type><scope>import</scope></dependency><dependency><groupId>org.apache.commons</groupId><artifactId>commons-compress</artifactId><version>${COMMONS_COMPRESS_VERSION}</version></dependency><dependency><groupId>org.codehaus.plexus</groupId><artifactId>plexus-utils</artifactId><version>${PLEXUS_UTILS_VERSION}</version></dependency><dependency><groupId>org.iq80.snappy</groupId><artifactId>snappy</artifactId><version>${SNAPPY_VERSION}</version></dependency></dependencies></dependencyManagement>|" pom.xml \
    && mvn -q -DskipTests -Dmaven.javadoc.skip=true package \
    && mv target/cantaloupe-*-SNAPSHOT.jar /cantaloupe.jar

FROM eclipse-temurin:25-jre-noble@sha256:2f1da100788559b397bcf48c736169ea5b070bde84e55f203bbee8e83d87a175

# Current security patches for the base's OS packages (temurin images lag
# Ubuntu's archive between rebuilds); versionless by necessity, upgrade-only.
# DL3008 (pin apt versions): Ubuntu's archive rotates point releases out,
# so a pinned bench-harness build would break on rebuild; upgrade-only is
# the recorded posture here (nothing from this image ships).
# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends libopenjp2-tools \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /usr/sbin/nologin cantaloupe \
    && mkdir -p /imageroot /var/cache/cantaloupe /opt/cantaloupe \
    && chown cantaloupe:cantaloupe /imageroot /var/cache/cantaloupe

# Static busybox for the healthcheck: the noble JRE base ships no curl/wget,
# and apt-installing one reintroduces multi-arch pin problems.
COPY --from=fetch /bin/busybox /busybox
COPY --from=build /cantaloupe.jar /opt/cantaloupe/cantaloupe.jar
# Default config: the decode-vs-decode posture. The bench harness mounts
# eval-cache-{on,off}.properties over this per pass.
COPY eval-cache-off.properties /opt/cantaloupe/cantaloupe.properties

USER cantaloupe

EXPOSE 8182

HEALTHCHECK --interval=10s --timeout=5s --retries=12 --start-period=90s \
    CMD ["/busybox", "wget", "-q", "-O", "/dev/null", "http://localhost:8182/health"]

CMD ["java", "-Dcantaloupe.config=/opt/cantaloupe/cantaloupe.properties", \
    "-jar", "/opt/cantaloupe/cantaloupe.jar"]
