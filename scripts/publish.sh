#!/bin/bash
# publish.sh - Publish WolfScale to GitHub
#
# Usage: ./scripts/publish.sh [commit-message]
#
# This script initializes git (if needed), adds all files,
# commits, and pushes to the WolfScale GitHub repository.

set -e

# Configuration
REPO_URL="${GITHUB_REPO:-git@github.com:$(git config user.name)/WolfScale.git}"
BRANCH="${BRANCH:-main}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

cd "$PROJECT_DIR"

# Check if git is installed
if ! command -v git &> /dev/null; then
    error "git is not installed. Please install git first."
fi

# Initialize git if not already initialized
if [ ! -d ".git" ]; then
    info "Initializing git repository..."
    git init
    git branch -M "$BRANCH"
fi

# Create .gitignore if it doesn't exist
if [ ! -f ".gitignore" ]; then
    info "Creating .gitignore..."
    cat > .gitignore << 'EOF'
# Build artifacts
/target/
*.rs.bk

# IDE files
.idea/
.vscode/
*.swp
*.swo
*~

# OS files
.DS_Store
Thumbs.db

# Local configuration
*.local.toml
local.toml

# Logs
*.log
logs/

# State and WAL data (production data)
/data/
*.db
*.db-journal

# Environment files
.env
.env.local
EOF
fi

# Check for remote
if ! git remote | grep -q "origin"; then
    info "Adding remote origin..."
    # Try to detect GitHub username
    GITHUB_USER=$(git config user.name 2>/dev/null || echo "")
    
    if [ -z "$GITHUB_USER" ]; then
        warn "Could not determine GitHub username."
        warn "Please set the remote manually:"
        echo "  git remote add origin git@github.com:YOUR_USERNAME/WolfScale.git"
        echo ""
        read -p "Enter your GitHub username: " GITHUB_USER
    fi
    
    REPO_URL="git@github.com:${GITHUB_USER}/WolfScale.git"
    git remote add origin "$REPO_URL"
    info "Remote set to: $REPO_URL"
fi

# Get commit message
if [ -n "$1" ]; then
    COMMIT_MSG="$1"
else
    COMMIT_MSG="Update WolfScale - $(date '+%Y-%m-%d %H:%M:%S')"
fi

# Stage all changes
info "Staging changes..."
git add -A

# Check if there are changes to commit
if git diff --cached --quiet; then
    warn "No changes to commit."
else
    info "Committing with message: $COMMIT_MSG"
    git commit -m "$COMMIT_MSG"
fi

# Push to remote
info "Pushing to origin/$BRANCH..."
git push -u origin "$BRANCH" 2>&1 || {
    warn "Push failed. Trying to set upstream and push again..."
    git push --set-upstream origin "$BRANCH"
}

info "✓ Successfully published to GitHub!"
echo ""
echo "Repository URL: https://github.com/${GITHUB_USER:-your-username}/WolfScale"
