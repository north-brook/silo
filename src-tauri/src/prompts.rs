pub(crate) fn git_push_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Review the current branch carefully before making changes:
- Inspect git status, staged changes, unstaged changes, and recent commits.
- Review the diff against the target branch.

Committing:
- Each commit should represent one logical unit of work — a single bug fix, a single feature addition, a single refactor. If a change touches unrelated things, split it into separate commits.
- Write commit messages in imperative mood ("Add search endpoint", not "Added search endpoint"). The subject line should complete the sentence "This commit will ___".
- Keep the subject line under 72 characters. If more context is needed, add a blank line followed by a body that explains *why* the change was made, not *what* changed (the diff shows that).
- Look at the recent commit history to match the style and conventions of the repository.
- Run focused validation (tests, linting, type checking) when appropriate and mention what you ran in your final summary.

Safety:
- Before staging files, review what you are about to commit. Do not commit secrets, credentials, API keys, tokens, .env files, or other sensitive material. Do not commit large binaries, build artifacts, or generated files that should be in .gitignore.
- If you discover sensitive material in the working tree, leave it unstaged and flag it in your summary.

Push the branch to origin without force pushing.

Constraints:
- Do not create or modify a pull request.
- Do not merge anything.
- Do not use force push or force-with-lease.
- If there is nothing to commit or nothing new to push, explain that clearly instead of inventing work.

When finished, print a concise summary of the commits you created, what you pushed, and any validation you ran."#,
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
