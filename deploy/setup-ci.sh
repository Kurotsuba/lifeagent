#!/usr/bin/env bash
# One-time setup that lets .github/workflows/ci.yml deploy (docs/HACKATHON.md §1 step 6):
# a Workload Identity Federation pool that trusts GitHub's OIDC tokens for pushes to
# g-hackathon of this repository only, and a deployer service account that the workflow
# impersonates. No service account key is created. Safe to run again.
set -euo pipefail

PROJECT="${PROJECT:-human-health-handler}"
REPO="${REPO:-Kurotsuba/lifeagent}"
BRANCH="${BRANCH:-g-hackathon}"
POOL=github
PROVIDER=github
CI_SA_NAME=lifeagent-ci
CI_SA="$CI_SA_NAME@$PROJECT.iam.gserviceaccount.com"
RUN_SA="lifeagent-run@$PROJECT.iam.gserviceaccount.com"

NUMBER="$(gcloud projects describe "$PROJECT" --format 'value(projectNumber)')"
# Cloud Build runs builds as the default compute service account (see `gcloud builds describe`).
BUILD_SA="$NUMBER-compute@developer.gserviceaccount.com"

gcloud services enable iamcredentials.googleapis.com sts.googleapis.com --project "$PROJECT"

if ! gcloud iam workload-identity-pools describe "$POOL" --project "$PROJECT" \
  --location global >/dev/null 2>&1; then
  gcloud iam workload-identity-pools create "$POOL" --project "$PROJECT" --location global \
    --display-name "GitHub Actions"
fi

# The condition is checked on every token exchange, so a workflow on another branch, a fork or
# another repository can't get credentials even if it names this provider.
CONDITION="assertion.repository == '$REPO' && assertion.ref == 'refs/heads/$BRANCH'"
MAPPING="google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.ref=assertion.ref"
if gcloud iam workload-identity-pools providers describe "$PROVIDER" --project "$PROJECT" \
  --location global --workload-identity-pool "$POOL" >/dev/null 2>&1; then
  gcloud iam workload-identity-pools providers update-oidc "$PROVIDER" --project "$PROJECT" \
    --location global --workload-identity-pool "$POOL" \
    --attribute-mapping "$MAPPING" --attribute-condition "$CONDITION"
else
  gcloud iam workload-identity-pools providers create-oidc "$PROVIDER" --project "$PROJECT" \
    --location global --workload-identity-pool "$POOL" \
    --issuer-uri "https://token.actions.githubusercontent.com" \
    --attribute-mapping "$MAPPING" --attribute-condition "$CONDITION"
fi

if ! gcloud iam service-accounts describe "$CI_SA" --project "$PROJECT" >/dev/null 2>&1; then
  gcloud iam service-accounts create "$CI_SA_NAME" --project "$PROJECT" \
    --display-name "LifeAgent GitHub Actions deployer"
fi

gcloud iam service-accounts add-iam-policy-binding "$CI_SA" --project "$PROJECT" \
  --role roles/iam.workloadIdentityUser \
  --member "principalSet://iam.googleapis.com/projects/$NUMBER/locations/global/workloadIdentityPools/$POOL/attribute.repository/$REPO" \
  --format none

# What deploy/deploy.sh build + deploy need. roles/viewer is for streaming the build log: with
# the default (legacy) logs bucket, `gcloud builds submit` refuses to stream it otherwise.
for role in roles/cloudbuild.builds.editor roles/storage.admin roles/artifactregistry.writer \
  roles/run.admin roles/serviceusage.serviceUsageConsumer roles/viewer; do
  gcloud projects add-iam-policy-binding "$PROJECT" --member "serviceAccount:$CI_SA" \
    --role "$role" --condition None --format none
done

# Deploying acts as the Cloud Run runtime account, and submitting a build as the build account.
for sa in "$RUN_SA" "$BUILD_SA"; do
  gcloud iam service-accounts add-iam-policy-binding "$sa" --project "$PROJECT" \
    --member "serviceAccount:$CI_SA" --role roles/iam.serviceAccountUser --format none
done

echo "workload_identity_provider: projects/$NUMBER/locations/global/workloadIdentityPools/$POOL/providers/$PROVIDER"
echo "service_account: $CI_SA"
