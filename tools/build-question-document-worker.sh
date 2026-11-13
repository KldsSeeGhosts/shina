#!/bin/sh
# Mechanically bundle KSG's unchanged parser, with its exact installed dependencies.
# Usage: sh tools/build-question-document-worker.sh /path/to/ksgcode /path/to/esbuild
set -eu
reference=$1
bundler=$2
"$bundler" "$reference/apps/server/src/provider/documentTextWorker.ts" \
  --platform=node --bundle --format=cjs --legal-comments=linked \
  --outfile=assets/question_document_worker.cjs \
  '--footer:js=module.exports.extractDocumentText(process.argv[2],process.argv[3]).then(text=>process.stdout.write(text)).catch(error=>{process.exitCode=error && error.code && /^E[A-Z0-9]+$/.test(String(error.code))?1:2;});'
