#!/usr/bin/env bash
# Run the ignored real-service QA smoke tests against a live Ollama plus
# throwaway Qdrant/MinIO containers.
#
#   OLLAMA_BASE_URL=http://10.124.124.3:11434/v1 ./scripts/run-qa-smoke.sh
#
# Optional overrides: QA_MODEL, EMBEDDING_MODEL, EMBEDDING_DIMENSION,
# QDRANT_HTTP_PORT, QDRANT_GRPC_PORT, MINIO_PORT, QDRANT_IMAGE, MINIO_IMAGE,
# MC_IMAGE.
set -euo pipefail

: "${OLLAMA_BASE_URL:?set OLLAMA_BASE_URL, e.g. http://10.124.124.3:11434/v1}"

QA_MODEL="${QA_MODEL:-qwen3.5:9b}"
EMBEDDING_MODEL="${EMBEDDING_MODEL:-bge-m3}"
EMBEDDING_DIMENSION="${EMBEDDING_DIMENSION:-1024}"

QDRANT_HTTP_PORT="${QDRANT_HTTP_PORT:-16333}"
QDRANT_GRPC_PORT="${QDRANT_GRPC_PORT:-16334}"
MINIO_PORT="${MINIO_PORT:-19000}"
QDRANT_IMAGE="${QDRANT_IMAGE:-qdrant/qdrant:latest}"
MINIO_IMAGE="${MINIO_IMAGE:-quay.io/minio/minio:latest}"
MC_IMAGE="${MC_IMAGE:-quay.io/minio/mc:latest}"

QDRANT_NAME="knota-qa-smoke-qdrant"
MINIO_NAME="knota-qa-smoke-minio"
BUCKET="knota-fold-test-smoke"

cleanup() {
  docker rm -f "$QDRANT_NAME" "$MINIO_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

command -v docker >/dev/null 2>&1 || {
  echo "docker is required" >&2
  exit 1
}

cleanup

echo "starting Qdrant (gRPC :$QDRANT_GRPC_PORT)"
docker run -d --name "$QDRANT_NAME" \
  -p "$QDRANT_HTTP_PORT:6333" -p "$QDRANT_GRPC_PORT:6334" \
  "$QDRANT_IMAGE" >/dev/null
for _ in $(seq 1 30); do
  if curl -fsS "http://localhost:$QDRANT_HTTP_PORT/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://localhost:$QDRANT_HTTP_PORT/healthz" >/dev/null || {
  docker logs "$QDRANT_NAME"
  exit 1
}

echo "starting MinIO (:$MINIO_PORT)"
docker run -d --name "$MINIO_NAME" \
  -p "$MINIO_PORT:9000" \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  "$MINIO_IMAGE" server /data >/dev/null
for _ in $(seq 1 30); do
  if curl -fsS "http://localhost:$MINIO_PORT/minio/health/ready" >/dev/null 2>&1; then
    break
  fi
  sleep 2
done
docker run --rm --network host --entrypoint /bin/sh "$MC_IMAGE" -c "
  mc alias set local http://localhost:$MINIO_PORT minioadmin minioadmin >/dev/null;
  mc mb --ignore-existing local/$BUCKET >/dev/null;"

KNOWLEDGE_BASE_ENABLED=true \
EMBEDDING_BASE_URL="$OLLAMA_BASE_URL" \
EMBEDDING_MODEL="$EMBEDDING_MODEL" \
EMBEDDING_DIMENSION="$EMBEDDING_DIMENSION" \
QA_LLM_BASE_URL="$OLLAMA_BASE_URL" \
QA_LLM_MODEL="$QA_MODEL" \
QA_LLM_PROVIDER=ollama \
QDRANT_URL="http://localhost:$QDRANT_GRPC_PORT" \
S3_ENDPOINT="http://localhost:$MINIO_PORT" \
S3_ACCESS_KEY=minioadmin \
S3_SECRET_KEY=minioadmin \
S3_TEST_BUCKET="$BUCKET" \
S3_FORCE_PATH_STYLE=true \
S3_REGION=us-east-1 \
cargo test --test mod -- requests::qa_smoke --ignored --test-threads=1 --nocapture
