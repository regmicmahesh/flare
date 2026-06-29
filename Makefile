.PHONY: dev-api dev-ui test lint fmt build docker-up docker-down

dev-api:
	cd backend && cargo run

dev-ui:
	cd frontend && npm run dev

test:
	cd backend && cargo test --all
	cd frontend && npm ci && npm run build

lint:
	cd backend && cargo fmt --all -- --check
	cd backend && cargo clippy --all-targets -- -D warnings
	cd frontend && npm ci && npm run lint --if-present

fmt:
	cd backend && cargo fmt --all

build:
	cd backend && cargo build --release
	cd frontend && npm ci && npm run build

docker-up:
	docker compose up --build -d

docker-down:
	docker compose down
