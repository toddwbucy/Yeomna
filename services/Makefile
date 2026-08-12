PROTO_DIR := ../proto
GEN_DIR := generated
# Prefer this project's venv interpreter when present so targets work from a
# cold shell (no activated venv); override with `make PYTHON=python ...`.
PYTHON := $(if $(wildcard .venv/bin/python),.venv/bin/python,python)

.PHONY: all proto-gen clean start-extraction stop-extraction

all: proto-gen

# Generate Python gRPC stubs from this repo's proto definitions. The wire
# package is yeomna.extraction, shared with the Rust client by construction.
proto-gen:
	@mkdir -p $(GEN_DIR)/yeomna/extraction
	$(PYTHON) -m grpc_tools.protoc \
		-I$(PROTO_DIR) \
		--python_out=$(GEN_DIR) \
		--grpc_python_out=$(GEN_DIR) \
		$(PROTO_DIR)/yeomna/extraction/extraction.proto

clean:
	rm -rf $(GEN_DIR)/yeomna

start-extraction:
	$(PYTHON) -m extraction.server &

stop-extraction:
	pkill -f "extraction.server" || true
