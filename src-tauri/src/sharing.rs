//! Code sharing (SPEC.md, section 13, Code sharing): whether a repository's
//! code may be sent to a model provider, asked once for each repository and
//! provider before a run or an explanation first sends it there. Runs and
//! explanations share one `CodeSharingService`; Settings → Code Sharing
//! lists and changes the answers.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Connection};

use crate::db::{self, Db};
use crate::models::{
    now_rfc3339, AgentProvider, AnswerCodeSharingRequest, AppError, AppResult, CodeAnswer,
    CodeAnswerScope, CodeConsent, CodeConsentState, CodeSharingQuestion, ErrorCode,
    RepositoryWorkspace,
};

/// Told when an answer changes, so open dialogs and Settings reload.
pub type SharingEmitter = Arc<dyn Fn() + Send + Sync>;

pub struct CodeSharingService {
    core: Db,
    emitter: SharingEmitter,
}

impl CodeSharingService {
    pub fn new(core: Db, emitter: SharingEmitter) -> Arc<Self> {
        Arc::new(CodeSharingService { core, emitter })
    }

    /// The workspaces a repository is in.
    pub async fn workspaces_of(&self, repository_id: &str) -> AppResult<Vec<RepositoryWorkspace>> {
        let repository = repository_id.to_string();
        self.core
            .call(move |conn| {
                let members = db::list_members(conn, None)?;
                let workspaces = db::list_workspaces(conn)?;
                Ok(workspaces
                    .into_iter()
                    .filter(|w| {
                        members.iter().any(|m| {
                            m.workspace_id == w.id
                                && m.repository_id.as_deref() == Some(repository.as_str())
                        })
                    })
                    .map(|w| RepositoryWorkspace {
                        id: w.id,
                        name: w.name,
                    })
                    .collect())
            })
            .await
    }

    /// The repository's answer for a provider: its own, else its
    /// workspaces', where a No wins over a Yes.
    pub async fn consent(
        &self,
        repository_id: &str,
        provider: AgentProvider,
        workspaces: &[RepositoryWorkspace],
    ) -> AppResult<CodeConsent> {
        let answers = self.core.call(|conn| answers(conn)).await?;
        Ok(decide(&answers, repository_id, provider, workspaces))
    }

    /// What New run and Explain need to ask: the answer for each provider
    /// that has an agent, and the workspaces to answer for.
    pub async fn question(
        &self,
        repository_id: &str,
        providers: &[AgentProvider],
    ) -> AppResult<CodeSharingQuestion> {
        let workspaces = self.workspaces_of(repository_id).await?;
        let answers = self.core.call(|conn| answers(conn)).await?;
        let consents = providers
            .iter()
            .map(|p| decide(&answers, repository_id, *p, &workspaces))
            .collect();
        Ok(CodeSharingQuestion {
            consents,
            workspaces,
        })
    }

    /// Refuses to send a repository's code to a provider unless it is
    /// answered Yes; what a run or an explanation checks before anything
    /// is copied.
    pub async fn require(&self, repository_id: &str, provider: AgentProvider) -> AppResult<()> {
        let workspaces = self.workspaces_of(repository_id).await?;
        match self.consent(repository_id, provider, &workspaces).await?.state {
            CodeConsentState::Allowed => Ok(()),
            CodeConsentState::Denied => Err(AppError::validation(format!(
                "This repository is answered No for sending its code to {}. Change it in Settings → Code Sharing.",
                provider.name()
            ))),
            // Not an invalid request but a step not taken yet: the dialog asks.
            CodeConsentState::Unasked => Err(AppError::new(
                ErrorCode::Conflict,
                format!(
                    "Answer whether this repository's code may be sent to {} first.",
                    provider.name()
                ),
            )),
        }
    }

    /// The question's answer, from New run, Explain, or Settings.
    pub async fn answer(&self, request: AnswerCodeSharingRequest) -> AppResult<()> {
        let (scope, scope_id) = match &request.workspace_id {
            Some(w) => (CodeAnswerScope::Workspace, w.clone()),
            None => (CodeAnswerScope::Repository, request.repository_id.clone()),
        };
        if scope_id.is_empty() {
            return Err(AppError::validation("Choose what the answer is for."));
        }
        let (provider, allowed) = (request.provider, request.allowed);
        self.core
            .call(move |conn| set_answer(conn, scope, &scope_id, provider, allowed))
            .await?;
        (self.emitter)();
        Ok(())
    }

    /// Settings → Code Sharing, **Ask again**: forget an answer, so the next
    /// run or explanation asks.
    pub async fn forget(
        &self,
        scope: CodeAnswerScope,
        scope_id: String,
        provider: AgentProvider,
    ) -> AppResult<()> {
        self.core
            .call(move |conn| remove_answer(conn, scope, &scope_id, provider))
            .await?;
        (self.emitter)();
        Ok(())
    }

    /// Settings → Code Sharing: every answer, named while its repository or
    /// workspace still exists.
    pub async fn list(&self) -> AppResult<Vec<CodeAnswer>> {
        self.core
            .call(|conn| {
                let mut answers = answers(conn)?;
                let repositories: HashMap<String, String> = db::list_repositories(conn)?
                    .into_iter()
                    .map(|r| {
                        let name = Path::new(&r.display_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or(r.display_path.clone());
                        (r.id, name)
                    })
                    .collect();
                let workspaces: HashMap<String, String> = db::list_workspaces(conn)?
                    .into_iter()
                    .map(|w| (w.id, w.name))
                    .collect();
                for answer in &mut answers {
                    answer.scope_name = match answer.scope {
                        CodeAnswerScope::Repository => repositories.get(&answer.scope_id).cloned(),
                        CodeAnswerScope::Workspace => workspaces.get(&answer.scope_id).cloned(),
                    };
                }
                Ok(answers)
            })
            .await
    }
}

/// The rule: the repository's own answer, else its workspaces', where a No
/// wins over a Yes.
fn decide(
    answers: &[CodeAnswer],
    repository_id: &str,
    provider: AgentProvider,
    workspaces: &[RepositoryWorkspace],
) -> CodeConsent {
    let state = |allowed: bool| {
        if allowed {
            CodeConsentState::Allowed
        } else {
            CodeConsentState::Denied
        }
    };
    let own = answers.iter().find(|a| {
        a.scope == CodeAnswerScope::Repository
            && a.scope_id == repository_id
            && a.provider == provider
    });
    if let Some(own) = own {
        return CodeConsent {
            provider,
            state: state(own.allowed),
            workspace_name: None,
        };
    }
    let of_workspaces: Vec<(&RepositoryWorkspace, bool)> = workspaces
        .iter()
        .filter_map(|w| {
            answers
                .iter()
                .find(|a| {
                    a.scope == CodeAnswerScope::Workspace
                        && a.scope_id == w.id
                        && a.provider == provider
                })
                .map(|a| (w, a.allowed))
        })
        .collect();
    let pick = of_workspaces
        .iter()
        .find(|(_, allowed)| !allowed)
        .or_else(|| of_workspaces.first());
    match pick {
        Some((w, allowed)) => CodeConsent {
            provider,
            state: state(*allowed),
            workspace_name: Some(w.name.clone()),
        },
        None => CodeConsent {
            provider,
            state: CodeConsentState::Unasked,
            workspace_name: None,
        },
    }
}

// ---------------------------------------------------------------------------
// brainiac.db: code_answers
// ---------------------------------------------------------------------------

fn answers(conn: &Connection) -> AppResult<Vec<CodeAnswer>> {
    let mut stmt = conn.prepare(
        "SELECT scope, scope_id, provider, allowed, answered_at FROM code_answers
         ORDER BY scope, scope_id, provider",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CodeAnswer {
                scope: db::parse_enum(r.get(0)?)?,
                scope_id: r.get(1)?,
                scope_name: None,
                provider: db::parse_enum(r.get(2)?)?,
                allowed: r.get(3)?,
                answered_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn set_answer(
    conn: &Connection,
    scope: CodeAnswerScope,
    scope_id: &str,
    provider: AgentProvider,
    allowed: bool,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO code_answers (scope, scope_id, provider, allowed, answered_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (scope, scope_id, provider)
         DO UPDATE SET allowed = excluded.allowed, answered_at = excluded.answered_at",
        params![
            db::enum_name(scope)?,
            scope_id,
            provider.as_str(),
            allowed,
            now_rfc3339()
        ],
    )?;
    Ok(())
}

fn remove_answer(
    conn: &Connection,
    scope: CodeAnswerScope,
    scope_id: &str,
    provider: AgentProvider,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM code_answers WHERE scope = ?1 AND scope_id = ?2 AND provider = ?3",
        params![db::enum_name(scope)?, scope_id, provider.as_str()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(scope: CodeAnswerScope, id: &str, allowed: bool) -> CodeAnswer {
        CodeAnswer {
            scope,
            scope_id: id.into(),
            scope_name: None,
            provider: AgentProvider::Anthropic,
            allowed,
            answered_at: String::new(),
        }
    }

    fn workspace(id: &str) -> RepositoryWorkspace {
        RepositoryWorkspace {
            id: id.into(),
            name: id.to_uppercase(),
        }
    }

    #[test]
    fn the_repositorys_own_answer_wins_and_a_workspace_no_wins_over_a_yes() {
        let (a, b) = (workspace("a"), workspace("b"));
        let both = [a.clone(), b.clone()];
        let unasked = decide(&[], "r", AgentProvider::Anthropic, &both);
        assert_eq!(unasked.state, CodeConsentState::Unasked);

        let yes_no = [
            answer(CodeAnswerScope::Workspace, "a", true),
            answer(CodeAnswerScope::Workspace, "b", false),
        ];
        let c = decide(&yes_no, "r", AgentProvider::Anthropic, &both);
        assert_eq!(c.state, CodeConsentState::Denied);
        assert_eq!(c.workspace_name.as_deref(), Some("B"));

        let mut own = yes_no.to_vec();
        own.push(answer(CodeAnswerScope::Repository, "r", true));
        let c = decide(&own, "r", AgentProvider::Anthropic, &both);
        assert_eq!(c.state, CodeConsentState::Allowed);
        assert_eq!(c.workspace_name, None);

        // Another provider is asked on its own.
        let c = decide(&own, "r", AgentProvider::Openai, &both);
        assert_eq!(c.state, CodeConsentState::Unasked);
    }
}
