# Builds a drop-in EPP image that adds the endpoint-score-observer picker plugin.
# Mirrors upstream llm-d Dockerfile.epp (distroless/static:nonroot, same ports/entrypoint),
# but builds THIS module's main.go, which blank-registers our plugin then runs the stock runner.
#
# Build (on a Go+docker box, e.g. etri-001):
#   docker build -t <registry>/epp-score-observer:<tag> .
#   docker push  <registry>/epp-score-observer:<tag>
ARG BASE_IMAGE=gcr.io/distroless/static:nonroot
FROM --platform=${BUILDPLATFORM} golang:1.25.11 AS go-builder
ARG TARGETOS
ARG TARGETARCH
WORKDIR /workspace
COPY go.mod go.mod
COPY go.sum go.sum
RUN go mod download
COPY main.go main.go
COPY pkg/ pkg/
ARG LDFLAGS="-s -w"
RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} go build \
    -ldflags="${LDFLAGS}" -o bin/epp .
FROM ${BASE_IMAGE}
WORKDIR /
COPY --from=go-builder /workspace/bin/epp /app/epp
USER 65532:65532
EXPOSE 9002
EXPOSE 9003
EXPOSE 9090
EXPOSE 5557
ENTRYPOINT ["/app/epp"]
