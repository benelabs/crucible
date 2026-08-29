# Instructions to Push to benelabs/crucible

## Current Setup
- ✅ Remote `origin` is configured to point to: `https://github.com/benelabs/crucible.git`
- ✅ Feature branch `feat/909-911-912-913-comprehensive-improvements` is ready to push
- ✅ All code and commits are prepared
- ❌ Currently getting 403 Permission Denied (milah-247 user doesn't have benelabs access)

## What You Need to Do

### Step 1: Authenticate as benelabs user (one of these options)

**Option A: Using GitHub Personal Access Token (PAT) - RECOMMENDED**
```bash
# Create a Personal Access Token at: https://github.com/settings/tokens/new
# Required scopes: repo, workflow

# Then use it to push:
git push https://<GITHUB_USERNAME>:<PERSONAL_ACCESS_TOKEN>@github.com/benelabs/crucible.git feat/909-911-912-913-comprehensive-improvements
```

**Option B: Using GitHub CLI (if you have it and are logged in as benelabs user)**
```bash
gh auth login  # Login as benelabs user
gh repo set-default benelabs/crucible
git push -u origin feat/909-911-912-913-comprehensive-improvements
```

**Option C: Using SSH Key (if benelabs account SSH key is configured)**
```bash
git remote set-url origin git@github.com:benelabs/crucible.git
git push -u origin feat/909-911-912-913-comprehensive-improvements
```

**Option D: Store credentials in git**
```bash
# When prompted, enter benelabs GitHub username and token/password
git push -u origin feat/909-911-912-913-comprehensive-improvements
# Git will ask for credentials and store them
```

---

## Current Status

```bash
# View current configuration:
git remote -v
# Output:
# benelabs  https://github.com/benelabs/crucible.git (fetch)
# benelabs  https://github.com/benelabs/crucible.git (push)
# origin    https://github.com/benelabs/crucible.git (fetch)
# origin    https://github.com/benelabs/crucible.git (push)

# View branch to push:
git log --oneline -5 feat/909-911-912-913-comprehensive-improvements
# Shows commits ready for push

# Check git status:
git status
# Should show: "working tree clean"
```

---

## Once Authenticated: Execute This

```bash
# Push the feature branch to benelabs
git push -u origin feat/909-911-912-913-comprehensive-improvements

# Create the PR on benelabs repository
gh pr create --repo benelabs/crucible \
  --title "feat: Comprehensive platform improvements (#909, #911, #912, #913)" \
  --body-file PR_DESCRIPTION_909_911_912_913.md \
  --base main \
  --head feat/909-911-912-913-comprehensive-improvements
```

---

## What Gets Pushed

**Branch:** `feat/909-911-912-913-comprehensive-improvements`

**Commits (2):**
1. `feat(#909): Interactive Challenge Engine with step progression and hint system`
   - 240 lines: InteractiveChallengeEngine.tsx
   - 360 lines: InteractiveChallengeEngine.css
   - 294 lines: InteractiveChallengeEngine.test.tsx
   - Plus vite.config.ts, App.tsx, App.css updates
   - Plus E2E tests, Sandbox files, and security tests

2. `docs: Add comprehensive PR description for issues #909, #911, #912, #913`
   - PR_DESCRIPTION_909_911_912_913.md (495 lines)

**Total:** 2,424+ lines across 14 files

**Issues Closed:** #909, #911, #912, #913

---

## Quick Copy-Paste Solution

If you have benelabs credentials, replace `USERNAME` and `TOKEN` and run:

```bash
git push https://USERNAME:TOKEN@github.com/benelabs/crucible.git feat/909-911-912-913-comprehensive-improvements
```

Or if you want to use SSH (assuming SSH key is set up):

```bash
git remote set-url origin git@github.com:benelabs/crucible.git
git push -u origin feat/909-911-912-913-comprehensive-improvements
```

---

## Verification After Push

```bash
# Verify branch is pushed
git branch -r | grep 909-911-912-913

# Check remote tracking
git branch -vv

# View pushed commits
git log origin/feat/909-911-912-913-comprehensive-improvements -3
```

---

## Need Help?

If you see authentication errors:
1. Check you're using correct benelabs credentials
2. Verify token has `repo` scope access
3. Try: `git credential-osxkeychain erase` (macOS) or `git credential reject` (Linux) to clear cached credentials
4. Ensure SSH key is added to GitHub account (if using SSH)

---

## Summary

✅ All code ready  
✅ Remote configured  
✅ Commits prepared  
⏳ Waiting for you to authenticate with benelabs credentials

**Next step:** Provide benelabs credentials (PAT, SSH key, or username) and run the push command above.
