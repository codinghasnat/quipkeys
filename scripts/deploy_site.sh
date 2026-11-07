#!/bin/sh
# Deploy site/ to Netlify (quipkeys.netlify.app) with the file-digest API.
# Needs a Netlify token: NETLIFY_AUTH_TOKEN, or a proxy that adds it.
set -e
SITE=92c0c63b-5d8b-4251-b549-0c6ca3382f07
cd "$(dirname "$0")/../site"
AUTH=${NETLIFY_AUTH_TOKEN:+-H "Authorization: Bearer $NETLIFY_AUTH_TOKEN"}
FILES=$(find . -type f | sed 's|^\./||')
JSON=$(for f in $FILES; do printf '"/%s":"%s",' "$f" "$(sha1sum "$f" | cut -d' ' -f1)"; done)
DEPLOY=$(curl -sS $AUTH -H "Content-Type: application/json" -d "{\"files\":{${JSON%,}}}" \
  "https://api.netlify.com/api/v1/sites/$SITE/deploys" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
for f in $FILES; do
  curl -sS $AUTH -X PUT -H "Content-Type: application/octet-stream" --data-binary "@$f" \
    "https://api.netlify.com/api/v1/deploys/$DEPLOY/files/$f" > /dev/null
done
echo "Deployed $DEPLOY to https://quipkeys.netlify.app"
