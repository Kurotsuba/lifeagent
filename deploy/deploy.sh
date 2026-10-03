#!/usr/bin/env bash
# Builds the image with Cloud Build, deploys it to Cloud Run, and points Cloud Scheduler at its
# tick endpoint (docs/HACKATHON.md §1). One-time project setup (APIs, service account, Cloud
# SQL, secrets) is the console checklist in docs/HACKATHON.md.
#
#   deploy/deploy.sh            # build + deploy + schedule
#   deploy/deploy.sh build      # only build and push the image
#   deploy/deploy.sh deploy     # only deploy the image for the current commit
#   deploy/deploy.sh schedule   # only create/update the Cloud Scheduler job
set -euo pipefail

PROJECT="${PROJECT:-human-health-handler}"
REGION="${REGION:-asia-northeast1}"
SERVICE=lifeagent
SQL_INSTANCE="$PROJECT:$REGION:lifeagent"
RUN_SA="lifeagent-run@$PROJECT.iam.gserviceaccount.com"

cd "$(dirname "$0")/.."
# TAG=<existing tag> reuses an image already built, e.g. to change only deploy/env.yaml:
#   TAG=12b98fe deploy/deploy.sh deploy
if [ -z "${TAG:-}" ]; then
  TAG="$(git rev-parse --short HEAD)"
  if [ -n "$(git status --porcelain)" ]; then
    TAG="$TAG-dirty"
  fi
fi
IMAGE="$REGION-docker.pkg.dev/$PROJECT/lifeagent/server:$TAG"

build() {
  # A release build of the server takes a while on the default 2-vCPU machine.
  gcloud builds submit . \
    --project "$PROJECT" \
    --tag "$IMAGE" \
    --machine-type e2-highcpu-8 \
    --timeout 3600s
}

deploy() {
  # One instance: per-user apps and live event streams are cached in the instance (see
  # docs/HACKATHON.md §1). Timeout 3600 s so an event stream lasts up to an hour per connection.
  gcloud run deploy "$SERVICE" \
    --project "$PROJECT" \
    --region "$REGION" \
    --image "$IMAGE" \
    --service-account "$RUN_SA" \
    --add-cloudsql-instances "$SQL_INSTANCE" \
    --set-secrets "LIFEAGENT_DB_PASSWORD=db-password:latest,LIFEAGENT_TICK_TOKEN=tick-token:latest" \
    --env-vars-file deploy/env.yaml \
    --allow-unauthenticated \
    --min-instances 0 \
    --max-instances 1 \
    --cpu 1 \
    --memory 512Mi \
    --timeout 3600
}

schedule() {
  local url token
  url="$(gcloud run services describe "$SERVICE" --project "$PROJECT" --region "$REGION" \
    --format 'value(status.url)')"
  token="$(gcloud secrets versions access latest --secret tick-token --project "$PROJECT")"
  local common=(
    --project "$PROJECT" --location "$REGION"
    --schedule "*/5 * * * *" --time-zone "Asia/Tokyo"
    --uri "$url/api/internal/tick" --http-method POST
    --attempt-deadline 600s
    # Otherwise gcloud prints the job, headers included, which puts the tick token in the log.
    --format none
  )
  if gcloud scheduler jobs describe lifeagent-tick --project "$PROJECT" --location "$REGION" \
    >/dev/null 2>&1; then
    gcloud scheduler jobs update http lifeagent-tick "${common[@]}" \
      --update-headers "X-LifeAgent-Tick-Token=$token"
  else
    gcloud scheduler jobs create http lifeagent-tick "${common[@]}" \
      --headers "X-LifeAgent-Tick-Token=$token"
  fi
}

case "${1:-all}" in
  build) build ;;
  deploy) deploy ;;
  schedule) schedule ;;
  all) build && deploy && schedule ;;
  *) echo "usage: $0 [build|deploy|schedule]" >&2; exit 2 ;;
esac
