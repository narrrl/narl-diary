.PHONY: dev dev-api dev-web build run check clean

# Two terminals: `make dev-api` and `make dev-web` (http://127.0.0.1:4243).
dev-api:
	cargo run

dev-web:
	cd web && bun run dev

# Everything, as one self-contained release binary at target/release/narl-diary.
build:
	cd web && bun install && bun run build
	cargo build --release

run: build
	./target/release/narl-diary

check:
	cargo clippy --all-targets -- -D warnings
	cargo test
	cd web && bun run check

clean:
	cargo clean
	rm -rf web/dist web/node_modules
