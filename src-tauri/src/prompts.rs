pub(crate) fn git_push_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Review the current branch carefully before making changes:
- Inspect git status, staged changes, unstaged changes, and recent commits.
- Review the diff against the target branch.

Staging:
- Stage ALL changes in the working tree — modified, deleted, and untracked files. Use `git add -A` to ensure nothing is missed.
- The only exception: do not stage secrets, credentials, API keys, tokens, .env files, or other sensitive material. Do not stage large binaries, build artifacts, or generated files that should be in .gitignore. If you discover such material, leave it unstaged and flag it in your summary.
- After staging, verify with `git status` that no intended changes remain unstaged. If anything was left out unintentionally, stage it before committing.

Committing:
- Organize staged changes into commits where each commit represents one logical unit of work — a single bug fix, a single feature addition, a single refactor. If the changes touch unrelated things, split them into separate commits. If the changes are cohesive, a single commit is fine.
- Write commit messages in imperative mood ("Add search endpoint", not "Added search endpoint"). The subject line should complete the sentence "This commit will ___".
- Keep the subject line under 72 characters. If more context is needed, add a blank line followed by a body that explains *why* the change was made, not *what* changed (the diff shows that).
- Look at the recent commit history to match the style and conventions of the repository.
- Run focused validation (tests, linting, type checking) when appropriate and mention what you ran in your final summary.

Push the branch to origin without force pushing.

Pull request:
- Check whether an open GitHub pull request already exists for this branch.
- If an open pull request exists, review its title and body against the current branch diff and update them if they are stale, vague, or no longer match the change.
- Keep PR updates focused on making the existing pull request accurate and reviewer-friendly. Do not open a new pull request from this flow.

Constraints:
- Do not merge anything.
- Do not create a new pull request.
- Do not use force push or force-with-lease.
- If there is nothing to commit or nothing new to push, explain that clearly instead of inventing work.

When finished, print a concise summary of the commits you created, what you pushed, any validation you ran, and any pull request updates you made."#,
        branch = branch,
        target_branch = target_branch,
    )
}

pub(crate) fn git_create_pr_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Prepare this branch for review and create an excellent pull request.

Review:
- Inspect git status, staged changes, unstaged changes, and the diff against the target branch.

Branch naming:
- If the current branch name is generic (e.g. "silo/nile", "silo/aabach" — a "silo/" prefix followed by a single placeholder word), rename it to something descriptive using `git branch -m silo/<descriptive-slug>`. The slug should be short, lowercase, hyphenated, and describe what the changes accomplish (e.g. "silo/superadmin-invites", "silo/fix-auth-redirect"). Do this before pushing.
- If the branch already has a descriptive name, skip renaming.

Committing:
- If there are uncommitted changes, organize them into commits where each commit represents one logical unit of work — a single bug fix, a single feature addition, a single refactor.
- Write commit messages in imperative mood ("Add search endpoint", not "Added search endpoint"). The subject line should complete the sentence "This commit will ___".
- Keep the subject line under 72 characters. If more context is needed, add a blank line followed by a body that explains *why* the change was made.
- Look at the recent commit history to match the style and conventions of the repository.
- Run focused validation (tests, linting, type checking) when appropriate and include results in your final summary.

Safety:
- Before staging files, review what you are about to commit. Do not commit secrets, credentials, API keys, tokens, .env files, or other sensitive material. Do not commit large binaries, build artifacts, or generated files that should be in .gitignore.
- If you discover sensitive material in the working tree, leave it unstaged and flag it in your summary.

Push the branch to origin without force pushing.

Pull request:
- Create a GitHub pull request against "{target_branch}" using `gh`.
- If a pull request for this branch already exists, update it instead of creating a duplicate.
- The PR title should be short (under 70 characters), specific, and written in imperative mood. It should tell the reviewer *what this PR accomplishes* at a glance — not describe the process or restate the branch name. Good: "Add rate limiting to public API endpoints". Bad: "Updates and changes".
- The PR body should help a reviewer understand and evaluate the change without reading the diff first. Lead with *why* the change was made, then describe the approach and what specifically changed. Include how it was tested and note any risks or follow-ups worth calling out.

Constraints:
- Do not merge the PR.
- Do not use force push or force-with-lease.
- If there is nothing to commit, you may still create or update the PR based on the existing branch state.

When finished, print the PR URL and a concise summary of the commits, validation, and PR changes you made."#,
        branch = branch,
        target_branch = target_branch,
    )
}

pub(crate) fn git_resolve_conflicts_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Your goal is to resolve the pull request's merge conflicts so the branch can be merged cleanly.

Start by understanding the current state:
- Inspect git status, staged changes, unstaged changes, and recent commits.
- Review the diff against the target branch.
- Check whether there is already an in-progress merge or other interrupted Git operation before starting new work.

Prepare local work safely:
- If there are intended local changes that are not committed yet, organize them into logical commits before resolving conflicts.
- Stage all intended changes with `git add -A`, excluding secrets, credentials, tokens, .env files, large binaries, build artifacts, or generated files that should remain untracked.
- Match the repository's existing commit message style.

Resolve the branch conflict:
- Fetch the latest target branch from origin.
- Merge `origin/{target_branch}` into the current branch. Prefer a merge-based repair for this flow; do not rebase or rewrite published history.
- If conflicts occur, resolve them carefully by understanding both sides of each change. Preserve important behavior from both branches whenever possible.
- Use file history, surrounding code, tests, and documentation to make informed resolutions instead of picking sides mechanically.
- Remove all conflict markers and make sure Git reports no unresolved paths before continuing.
- If the branch already merges cleanly, say so clearly instead of inventing extra work.

Validation:
- Run focused validation for the affected areas after resolving conflicts.
- If validation fails, fix the underlying issue before pushing when reasonable.

Push and PR hygiene:
- Push the updated branch to origin without force pushing or force-with-lease.
- Review the existing pull request title and body. Update them if the conflict-resolution work changes reviewer expectations, testing notes, or rollout risks.

Constraints:
- Do not merge the pull request.
- Do not use rebase, force push, or force-with-lease.
- Do not discard user work to make the conflict disappear.
- If you hit a blocker you cannot safely resolve, stop and explain exactly what remains conflicted and why.

When finished, print a concise summary of the conflicts you resolved, any commits you created, the validation you ran, what you pushed, and any PR updates you made."#,
        branch = branch,
        target_branch = target_branch,
    )
}
