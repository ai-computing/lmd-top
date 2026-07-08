module github.com/etri/epp-score-observer

go 1.25.11

// The require version MUST match the llm-d endpoint-picker image you are replacing
// (ghcr.io/llm-d/llm-d-router-endpoint-picker-dev:main). Pin it on the build box with:
//
//   go get github.com/llm-d/llm-d-router@<commit-or-tag>
//   go mod tidy
//
// To find the commit baked into your running EPP image:
//   kubectl logs deploy/llmd-router-epp -n llm-serving | grep -i commit   # if logged
//   # or inspect the image labels / version endpoint.
//
// controller-runtime and prometheus client versions are resolved transitively from
// github.com/llm-d/llm-d-router's go.mod (go mod tidy will align them).
require (
	github.com/llm-d/llm-d-router v0.0.0-00010101000000-000000000000
	github.com/prometheus/client_golang v1.20.0
	sigs.k8s.io/controller-runtime v0.19.0
)
