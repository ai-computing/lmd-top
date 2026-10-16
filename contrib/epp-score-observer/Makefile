GO ?= go
IMAGE ?= lmd-epp-policy:dev
.PHONY: test replay build image

test:
	$(GO) test -mod=readonly ./...

replay:
	$(GO) run -mod=readonly ./cmd/replay examples/parameters.json examples/requests.json

build:
	mkdir -p bin
	CGO_ENABLED=0 $(GO) build -mod=readonly -trimpath -buildvcs=false -o bin/epp .

image:
	docker build -t $(IMAGE) .
