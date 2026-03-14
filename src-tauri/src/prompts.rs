pub(crate) fn git_push_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Review the current branch carefully before making changes:
- Inspect git status, staged changes, unstaged changes, and recent commits.
- Review the diff against the target branch.
- Group the work into a small, sensible series of human-readable commits. Prefer multiple focused commits over one large commit when that improves history.
- Write polished commit messages that explain intent clearly.
- Run focused validation when appropriate and mention what you ran in your final summary.
- Push the branch to origin without force pushing.

Constraints:
- Do not create or modify a pull request.
- Do not merge anything.
- Do not use force push or force-with-lease.
- If there is nothing to commit or nothing new to push, explain that clearly instead of inventing work.

When finished, print a concise summary of the commits you created or reused, what you pushed, and any validation you ran."#,
        branch = branch,
        target_branch = target_branch,
    )
}

pub(crate) fn git_create_pr_prompt(branch: &str, target_branch: &str) -> String {
    format!(
        r#"You are operating inside the workspace repository on branch "{branch}" targeting "{target_branch}".

Prepare this branch for review and create an excellent pull request:
- Inspect git status, staged changes, unstaged changes, and the diff against the target branch.
- If there are uncommitted changes, organize them into a sensible series of human-readable commits.
- Write polished commit messages that make the history easy for another engineer to understand.
- Run focused validation when appropriate and include the important results in your final summary.
- Push the branch to origin without force pushing.
- Create a GitHub pull request against "{target_branch}" using `gh`.
- If a pull request for this branch already exists, update it instead of creating a duplicate.
- Write a beautiful, useful PR title and body with clear context, a high-signal summary of changes, testing notes, and any meaningful risks or follow-ups.

Constraints:
- Do not merge the PR.
- Do not use force push or force-with-lease.
- If there is nothing to commit, you may still create or update the PR based on the existing branch state.

When finished, print the PR URL and a concise summary of the commits, validation, and PR changes you made."#,
        branch = branch,
        target_branch = target_branch,
    )
}
