//! User-facing strings, centralized like gitui's `strings` module.

pub struct Title {
    pub status: &'static str,
    pub log: &'static str,
    pub log_detail: &'static str,
    pub help: &'static str,
    pub blame: &'static str,
    pub confirm: &'static str,
    pub message: &'static str,
    pub file_history: &'static str,
    pub file_finder: &'static str,
}

pub const TITLE: Title = Title {
    status: "Files (svn status)",
    log: "Log (svn log)",
    log_detail: "Revision details",
    help: "Help",
    blame: "Blame",
    confirm: "Confirm",
    message: "Message",
    file_history: "File history",
    file_finder: "Find file",
};

pub struct Msg {
    pub loading: &'static str,
    pub no_working_copy: &'static str,
    pub empty_log: &'static str,
    pub empty_status: &'static str,
    pub commit_all: &'static str,
    pub commit_staged: &'static str,
    pub revert_confirm: &'static str,
    pub update_confirm: &'static str,
    pub resolve_confirm: &'static str,
    pub update_to_rev_confirm: &'static str,
    pub add_done: &'static str,
    pub revert_done: &'static str,
    pub resolve_done: &'static str,
}

pub const MSG: Msg = Msg {
    loading: "Loading...",
    no_working_copy: "Error: not an SVN working copy (svn info failed)",
    empty_log: "No revisions found",
    empty_status: "Working copy is clean",
    commit_all: "Commit all changes?",
    commit_staged: "Commit staged changes?",
    revert_confirm: "Revert local changes? (discards modifications)",
    update_confirm: "Run svn update?",
    resolve_confirm: "Resolve conflict using the working copy version?",
    update_to_rev_confirm: "Update working copy to selected revision?",
    add_done: "Added to version control",
    revert_done: "Reverted",
    resolve_done: "Conflict resolved",
};
