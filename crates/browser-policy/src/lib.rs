//! Fail-closed agent authorization. This crate deliberately cannot execute web actions.
//! The caller supplies a trusted clock and fresh engine identity at every check.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Inspect,
    Capture,
    Navigate,
    Click,
    Type,
    Select,
    Scroll,
    History,
    Wait,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub workspace: String,
    pub pane: String,
    pub tab: String,
    pub container: String,
    pub pane_generation: u64,
    pub navigation_generation: u64,
    pub document_generation: u64,
    pub origin: String,
    pub account_epoch: u64,
}
impl Context {
    pub fn validate(&self) -> Result<(), Denied> {
        if [&self.workspace, &self.pane, &self.tab, &self.container]
            .iter()
            .any(|s| s.is_empty() || s.len() > 128)
        {
            return Err(Denied::Invalid);
        }
        if canonical_origin(&self.origin)? != self.origin {
            return Err(Denied::Origin);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub session: String,
    pub recipient: String,
    pub context: Context,
    pub capabilities: BTreeSet<Capability>,
    pub expires_at_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Reversible,
    ExternalSend,
    Submission,
    Destructive,
    Permission,
    Account,
    Transfer,
    Payment,
    Credential,
}
impl Risk {
    fn needs_approval(self) -> bool {
        !matches!(self, Self::Read | Self::Reversible)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub operation: String,
    pub session: String,
    pub context: Context,
    pub capability: Capability,
    pub risk: Risk,
    pub target: String,
    pub data_digest: String,
    pub deadline_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub intent: Intent,
    pub expires_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lease {
    session: String,
    operation: String,
    expires_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Operation {
    intent: Intent,
    outcome: Option<Outcome>,
}
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Denied {
    #[error("agent features are disabled until native release gates pass")]
    Disabled,
    #[error("no matching explicit grant")]
    NoGrant,
    #[error("invalid request")]
    Invalid,
    #[error("request deadline or grant has expired")]
    Expired,
    #[error("document, navigation, account, or pane context changed")]
    Stale,
    #[error("origin is not authorized")]
    Origin,
    #[error("capability is not granted")]
    Capability,
    #[error("a different operation owns this tab")]
    Busy,
    #[error("specific approval is required")]
    Approval,
    #[error("payment and credential entry require manual control")]
    Manual,
    #[error("operation ID was already used; inspect the result before retrying")]
    Duplicate,
    #[error("container is paused for an account or storage change")]
    Barrier,
    #[error("too many live grants or operations")]
    Capacity,
}

/// All authority is transient. Never serialize Broker into workspace persistence.
#[derive(Debug, Default)]
pub struct Broker {
    enabled: bool,
    grants: BTreeMap<(String, String), Grant>,
    leases: BTreeMap<String, Lease>,
    operations: BTreeMap<String, Operation>,
    approvals: BTreeMap<String, Approval>,
    barriers: BTreeSet<String>,
}
impl Broker {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }
    /// Called only from trusted native UI after the user reviews recipient and exact tab.
    pub fn grant_from_user(&mut self, grant: Grant, now: u64) -> Result<(), Denied> {
        if !self.enabled {
            return Err(Denied::Disabled);
        }
        grant.context.validate()?;
        if grant.session.is_empty()
            || grant.recipient.trim().is_empty()
            || grant.capabilities.is_empty()
        {
            return Err(Denied::Invalid);
        }
        if grant.expires_at_ms <= now {
            return Err(Denied::Expired);
        }
        if self.grants.len() >= 1024 {
            return Err(Denied::Capacity);
        }
        self.grants
            .insert((grant.session.clone(), grant.context.tab.clone()), grant);
        Ok(())
    }
    pub fn granted_tabs(&self, session: &str, now: u64) -> Vec<&Context> {
        if !self.enabled {
            return vec![];
        }
        self.grants
            .values()
            .filter(|g| {
                g.session == session
                    && g.expires_at_ms > now
                    && !self.barriers.contains(&g.context.container)
            })
            .map(|g| &g.context)
            .collect()
    }
    pub fn authorize_read(
        &self,
        session: &str,
        fresh: &Context,
        capability: Capability,
        now: u64,
    ) -> Result<(), Denied> {
        if !matches!(
            capability,
            Capability::Inspect | Capability::Capture | Capability::Wait
        ) {
            return Err(Denied::Capability);
        }
        self.check_grant(session, fresh, capability, now)
    }
    fn check_grant(
        &self,
        session: &str,
        fresh: &Context,
        capability: Capability,
        now: u64,
    ) -> Result<(), Denied> {
        if !self.enabled {
            return Err(Denied::Disabled);
        }
        fresh.validate()?;
        if self.barriers.contains(&fresh.container) {
            return Err(Denied::Barrier);
        }
        let grant = self
            .grants
            .get(&(session.to_owned(), fresh.tab.clone()))
            .ok_or(Denied::NoGrant)?;
        if grant.expires_at_ms <= now {
            return Err(Denied::Expired);
        }
        if grant.context != *fresh {
            return Err(Denied::Stale);
        }
        if !grant.capabilities.contains(&capability) {
            return Err(Denied::Capability);
        }
        Ok(())
    }
    /// Freeze the entire intent; callers cannot reuse an approval with changed data.
    pub fn approve_from_user(&mut self, approval: Approval, now: u64) -> Result<(), Denied> {
        if !self.enabled {
            return Err(Denied::Disabled);
        }
        if approval.expires_at_ms <= now || approval.intent.deadline_ms <= now {
            return Err(Denied::Expired);
        }
        if matches!(approval.intent.risk, Risk::Payment | Risk::Credential) {
            return Err(Denied::Manual);
        }
        self.check_grant(
            &approval.intent.session,
            &approval.intent.context,
            approval.intent.capability,
            now,
        )?;
        self.approvals
            .insert(approval.intent.operation.clone(), approval);
        Ok(())
    }
    /// Invoke immediately before dispatch to CEF, using freshly read context.
    /// Authorization does not mean the action succeeded; finish must record observation.
    pub fn begin(&mut self, intent: Intent, fresh: &Context, now: u64) -> Result<(), Denied> {
        if intent.operation.is_empty()
            || intent.operation.len() > 128
            || intent.target.len() > 8192
            || intent.data_digest.len() > 128
        {
            return Err(Denied::Invalid);
        }
        if intent.deadline_ms <= now {
            return Err(Denied::Expired);
        }
        if intent.context != *fresh {
            return Err(Denied::Stale);
        }
        if matches!(
            intent.capability,
            Capability::Inspect | Capability::Capture | Capability::Wait
        ) {
            return Err(Denied::Capability);
        }
        if matches!(intent.risk, Risk::Payment | Risk::Credential) {
            return Err(Denied::Manual);
        }
        self.check_grant(&intent.session, fresh, intent.capability, now)?;
        if self.operations.contains_key(&intent.operation) {
            return Err(Denied::Duplicate);
        }
        if self.operations.len() >= 10000 {
            return Err(Denied::Capacity);
        }
        if intent.capability == Capability::Navigate {
            let target = Url::parse(&intent.target).map_err(|_| Denied::Invalid)?;
            if !matches!(target.scheme(), "https" | "http")
                || !target.username().is_empty()
                || target.password().is_some()
            {
                return Err(Denied::Origin);
            }
            if canonical_origin(&intent.target)? != fresh.origin {
                return Err(Denied::Origin);
            }
        }
        if intent.risk.needs_approval() {
            let approval = self
                .approvals
                .get(&intent.operation)
                .ok_or(Denied::Approval)?;
            if approval.intent != intent || approval.expires_at_ms <= now {
                return Err(Denied::Approval);
            }
        }
        // Expiry cannot imply that a possibly dispatched side effect is safe to retry.
        if let Some(lease) = self.leases.get(&fresh.tab) {
            if lease.expires_at_ms <= now {
                if let Some(op) = self.operations.get_mut(&lease.operation) {
                    op.outcome = Some(Outcome::Uncertain);
                }
                self.leases.remove(&fresh.tab);
            } else {
                return Err(Denied::Busy);
            }
        }
        let lease = Lease {
            session: intent.session.clone(),
            operation: intent.operation.clone(),
            expires_at_ms: intent.deadline_ms.min(now.saturating_add(30000)),
        };
        self.approvals.remove(&intent.operation);
        self.leases.insert(fresh.tab.clone(), lease);
        self.operations.insert(
            intent.operation.clone(),
            Operation {
                intent,
                outcome: None,
            },
        );
        Ok(())
    }
    pub fn finish(&mut self, operation: &str, outcome: Outcome) -> Result<(), Denied> {
        let op = self.operations.get_mut(operation).ok_or(Denied::Invalid)?;
        if op.outcome.is_some() {
            return Err(Denied::Duplicate);
        }
        op.outcome = Some(outcome);
        if self
            .leases
            .get(&op.intent.context.tab)
            .is_some_and(|l| l.operation == operation)
        {
            self.leases.remove(&op.intent.context.tab);
        }
        Ok(())
    }
    pub fn outcome(&self, operation: &str) -> Option<Option<&Outcome>> {
        self.operations.get(operation).map(|o| o.outcome.as_ref())
    }
    pub fn human_takeover(&mut self, tab: &str) {
        self.revoke_tab(tab);
    }
    pub fn revoke_tab(&mut self, tab: &str) {
        self.grants.retain(|(_, t), _| t != tab);
        self.approvals.retain(|_, a| a.intent.context.tab != tab);
        if let Some(lease) = self.leases.remove(tab) {
            if let Some(op) = self.operations.get_mut(&lease.operation) {
                op.outcome = Some(Outcome::Uncertain);
            }
        }
    }
    pub fn revoke_session(&mut self, session: &str) {
        let tabs: Vec<_> = self
            .grants
            .values()
            .filter(|g| g.session == session)
            .map(|g| g.context.tab.clone())
            .collect();
        self.grants.retain(|(s, _), _| s != session);
        self.approvals.retain(|_, a| a.intent.session != session);
        for tab in tabs {
            if self.leases.get(&tab).is_some_and(|l| l.session == session) {
                if let Some(lease) = self.leases.remove(&tab) {
                    if let Some(op) = self.operations.get_mut(&lease.operation) {
                        op.outcome = Some(Outcome::Uncertain);
                    }
                }
            }
        }
    }
    pub fn begin_container_barrier(&mut self, container: &str) {
        self.barriers.insert(container.into());
        let tabs: Vec<_> = self
            .grants
            .values()
            .filter(|g| g.context.container == container)
            .map(|g| g.context.tab.clone())
            .collect();
        for tab in tabs {
            self.revoke_tab(&tab);
        }
    }
    pub fn end_container_barrier(&mut self, container: &str) {
        self.barriers.remove(container);
    }
    pub fn stop_all(&mut self) {
        let tabs: Vec<_> = self.leases.keys().cloned().collect();
        for tab in tabs {
            self.revoke_tab(&tab);
        }
        self.grants.clear();
        self.approvals.clear();
    }
}
pub fn canonical_origin(input: &str) -> Result<String, Denied> {
    let u = Url::parse(input).map_err(|_| Denied::Origin)?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(Denied::Origin);
    }
    Ok(u.origin().ascii_serialization())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Camera,
    Microphone,
    Location,
    Notifications,
    ClipboardRead,
    ScreenCapture,
}
#[derive(Debug, Default)]
pub struct Permissions(BTreeMap<(String, String, Permission), bool>);
impl Permissions {
    pub fn set_from_user(
        &mut self,
        container: &str,
        origin: &str,
        permission: Permission,
        allow: bool,
    ) -> Result<(), Denied> {
        if container.is_empty() {
            return Err(Denied::Invalid);
        }
        // Screen selection is per-action through the OS, never a saved whole-display grant.
        if permission == Permission::ScreenCapture && allow {
            return Err(Denied::Manual);
        }
        self.0.insert(
            (container.into(), canonical_origin(origin)?, permission),
            allow,
        );
        Ok(())
    }
    pub fn allowed(&self, container: &str, origin: &str, permission: Permission) -> bool {
        canonical_origin(origin)
            .ok()
            .and_then(|o| self.0.get(&(container.into(), o, permission)))
            .copied()
            .unwrap_or(false)
    }
    pub fn revoke_origin(&mut self, container: &str, origin: &str) -> Result<(), Denied> {
        let o = canonical_origin(origin)?;
        self.0.retain(|(c, p, _), _| c != container || p != &o);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> Context {
        Context {
            workspace: "w".into(),
            pane: "p".into(),
            tab: "t".into(),
            container: "c".into(),
            pane_generation: 1,
            navigation_generation: 1,
            document_generation: 1,
            origin: "https://example.com".into(),
            account_epoch: 1,
        }
    }
    fn broker() -> Broker {
        let mut b = Broker::new(true);
        for s in ["a", "b"] {
            b.grant_from_user(
                Grant {
                    session: s.into(),
                    recipient: "local approved agent".into(),
                    context: ctx(),
                    capabilities: [Capability::Inspect, Capability::Click, Capability::Navigate]
                        .into(),
                    expires_at_ms: 100000,
                },
                0,
            )
            .unwrap();
        }
        b
    }
    fn intent(op: &str, session: &str) -> Intent {
        Intent {
            operation: op.into(),
            session: session.into(),
            context: ctx(),
            capability: Capability::Click,
            risk: Risk::Reversible,
            target: "snapshot:1/button:2".into(),
            data_digest: "none".into(),
            deadline_ms: 10000,
        }
    }
    #[test]
    fn disabled_is_default() {
        assert_eq!(
            Broker::default().authorize_read("a", &ctx(), Capability::Inspect, 0),
            Err(Denied::Disabled)
        );
    }
    #[test]
    fn no_cross_tab_leak() {
        let b = broker();
        let mut c = ctx();
        c.tab = "secret".into();
        assert_eq!(
            b.authorize_read("a", &c, Capability::Inspect, 0),
            Err(Denied::NoGrant)
        );
        assert!(b.granted_tabs("unknown", 0).is_empty());
    }
    #[test]
    fn screenshots_need_separate_grant() {
        assert_eq!(
            broker().authorize_read("a", &ctx(), Capability::Capture, 0),
            Err(Denied::Capability)
        );
    }
    #[test]
    fn only_one_mutator() {
        let mut b = broker();
        b.begin(intent("1", "a"), &ctx(), 0).unwrap();
        assert_eq!(b.begin(intent("2", "b"), &ctx(), 1), Err(Denied::Busy));
        b.finish("1", Outcome::Succeeded).unwrap();
        b.begin(intent("2", "b"), &ctx(), 2).unwrap();
    }
    #[test]
    fn generation_stale() {
        for field in 0..4 {
            let mut c = ctx();
            match field {
                0 => c.navigation_generation += 1,
                1 => c.document_generation += 1,
                2 => c.pane_generation += 1,
                _ => c.account_epoch += 1,
            };
            assert_eq!(broker().begin(intent("1", "a"), &c, 0), Err(Denied::Stale));
        }
    }
    #[test]
    fn takeover_revokes_reads_and_actions() {
        let mut b = broker();
        b.begin(intent("1", "a"), &ctx(), 0).unwrap();
        b.human_takeover("t");
        assert_eq!(
            b.authorize_read("a", &ctx(), Capability::Inspect, 1),
            Err(Denied::NoGrant)
        );
        assert_eq!(b.outcome("1"), Some(Some(&Outcome::Uncertain)));
    }
    #[test]
    fn approvals_freeze_data_and_redirects() {
        let mut b = broker();
        let mut i = intent("1", "a");
        i.risk = Risk::Submission;
        assert_eq!(b.begin(i.clone(), &ctx(), 0), Err(Denied::Approval));
        b.approve_from_user(
            Approval {
                intent: i.clone(),
                expires_at_ms: 99,
            },
            0,
        )
        .unwrap();
        i.data_digest = "changed".into();
        assert_eq!(b.begin(i.clone(), &ctx(), 1), Err(Denied::Approval));
        let mut c = ctx();
        c.origin = "https://evil.example".into();
        assert_eq!(b.begin(i, &c, 1), Err(Denied::Stale));
    }
    #[test]
    fn repeated_operation_never_replayed() {
        let mut b = broker();
        let i = intent("1", "a");
        b.begin(i.clone(), &ctx(), 0).unwrap();
        b.finish("1", Outcome::Uncertain).unwrap();
        assert_eq!(b.begin(i, &ctx(), 1), Err(Denied::Duplicate));
    }
    #[test]
    fn credentials_and_payment_manual() {
        for risk in [Risk::Payment, Risk::Credential] {
            let mut i = intent("1", "a");
            i.risk = risk;
            assert_eq!(broker().begin(i, &ctx(), 0), Err(Denied::Manual));
        }
    }
    #[test]
    fn container_barrier_revokes_all() {
        let mut b = broker();
        b.begin_container_barrier("c");
        assert_eq!(b.begin(intent("1", "a"), &ctx(), 0), Err(Denied::Barrier));
        b.end_container_barrier("c");
        assert_eq!(b.begin(intent("1", "a"), &ctx(), 0), Err(Denied::NoGrant));
    }
    #[test]
    fn origin_normalization_prevents_credential_tricks() {
        for url in [
            "file:///etc/passwd",
            "https://user@evil.com",
            "javascript:alert(1)",
        ] {
            assert!(canonical_origin(url).is_err());
        }
        assert_eq!(
            canonical_origin("https://EXAMPLE.com:443/a").unwrap(),
            "https://example.com"
        );
    }
    #[test]
    fn permissions_are_container_scoped_and_default_deny() {
        let mut p = Permissions::default();
        p.set_from_user("A", "https://example.com", Permission::Camera, true)
            .unwrap();
        assert!(p.allowed("A", "https://example.com", Permission::Camera));
        assert!(!p.allowed("B", "https://example.com", Permission::Camera));
        assert!(!p.allowed("A", "https://other.example", Permission::Camera));
        assert_eq!(
            p.set_from_user("A", "https://example.com", Permission::ScreenCapture, true),
            Err(Denied::Manual)
        );
        p.revoke_origin("A", "https://example.com").unwrap();
        assert!(!p.allowed("A", "https://example.com", Permission::Camera));
    }
}
