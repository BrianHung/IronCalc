#!/bin/bash
# Pre-commit hook for API parity checking
# Install with: ln -s ../../scripts/pre-commit-api-parity.sh .git/hooks/pre-commit

set -e

echo "🔍 Pre-commit: Checking API parity..."

# Get the root directory
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Check if any binding files have changed
CHANGED_FILES=$(git diff --cached --name-only | grep -E "(bindings/.*\.rs|bindings/.*\.ts)" || true)

if [ -z "$CHANGED_FILES" ]; then
    echo "✅ No binding files changed, skipping API parity check"
    exit 0
fi

echo "📝 Binding files changed:"
echo "$CHANGED_FILES"

# Run quick compilation check
echo "🔧 Checking compilation..."
if ! cargo check --manifest-path bindings/python/Cargo.toml --quiet; then
    echo "❌ Python binding compilation failed"
    exit 1
fi

if ! cargo check --manifest-path bindings/nodejs/Cargo.toml --quiet; then
    echo "❌ Node.js binding compilation failed"
    exit 1
fi

if ! cargo check --manifest-path bindings/wasm/Cargo.toml --quiet; then
    echo "❌ WASM binding compilation failed"
    exit 1
fi

echo "✅ All bindings compile successfully"

# Run API parity enforcement (warning mode for git hooks)
echo "🎯 Running API parity enforcement..."
python3 scripts/enforce_api_parity.py . --warning-mode
if [ $? -eq 0 ]; then
    echo "🚀 Commit proceeding!"
else
    echo "💥 Unexpected error in parity check"
    echo "🔄 Proceeding with commit anyway"
fi
exit 0