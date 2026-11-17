//! Agent tabs (spec 13 §7.3): opened in a group titled with the task, at
//! the end of the person's last active window, never activated, rendered
//! in the engine's agent host window; not in history, not restored, not in
//! ⌘⇧T. A person who shows one takes it over until handed back.

use mado_types::command::Origin;
use mado_types::patch::PatchRecorder;
use mado_types::state::{AgentTab, AgentTabStatus, TaskStatus, TaskSummary};
use mado_types::{ClientId, ErrorCode, MadoError, TabId, TaskId};

use super::Core;

impl Core {
    /// The task a tool call belongs to: the agent's, or for a control
    /// client its implicit task, one per client (14 §4).
    pub(super) fn task_of(&mut self, origin: &Origin, rec: &mut PatchRecorder) -> Option<TaskId> {
        match origin {
            Origin::Agent { task } => Some(*task),
            Origin::Control { client } | Origin::FullControl { client } => {
                Some(self.implicit_task(*client, rec))
            }
            _ => None,
        }
    }

    fn implicit_task(&mut self, client: ClientId, rec: &mut PatchRecorder) -> TaskId {
        if let Some(t) = self.control_tasks.get(&client) {
            return *t;
        }
        let task = TaskId::new();
        self.control_tasks.insert(client, task);
        let title = self
            .control_clients
            .get(&client)
            .map_or_else(|| "Control".to_owned(), |c| c.name.clone());
        self.state.tx(rec).ai().tasks().insert(
            task,
            TaskSummary {
                id: task,
                title,
                status: TaskStatus::Running,
                step: String::new(),
                group_id: None,
                cost_micros: 0,
            },
        );
        task
    }

    /// Opens `url` in a new agent tab of `task`.
    pub(super) fn agent_open(
        &mut self,
        task: TaskId,
        url: String,
        rec: &mut PatchRecorder,
    ) -> Result<TabId, MadoError> {
        let window = match self.current_window() {
            Some(w) => w,
            None => self.open_window(mado_types::state::Bounds::default(), &[])?,
        };
        let (now, prefs) = (self.now(), self.prefs());
        let tab = self
            .model
            .open_tab(window, url, None, false, None, now, prefs)?;
        if let Some(t) = self.model.tabs.get_mut(&tab) {
            t.agent = Some(AgentTab {
                task_id: task,
                status: AgentTabStatus::Working,
            });
        }
        let end = self
            .model
            .windows
            .get(&window)
            .map_or(0, |w| w.tab_order.len());
        let group = self
            .model
            .groups
            .values()
            .find(|g| g.agent_task_id == Some(task) && g.window_id == Some(window))
            .map(|g| g.id);
        match group {
            Some(g) => {
                self.model
                    .move_tabs(&[tab], Some(window), end, Some(Some(g)), now)?;
            }
            None => {
                let title = self
                    .state
                    .ai
                    .tasks
                    .get(&task)
                    .map(|t| t.title.clone())
                    .unwrap_or_default();
                let g = self.model.create_group(&[tab], Some(title), None, now)?;
                if let Some(group) = self.model.groups.get_mut(&g) {
                    group.agent_task_id = Some(task);
                    group.saved = false;
                }
                let end = self
                    .model
                    .windows
                    .get(&window)
                    .map_or(0, |w| w.tab_order.len());
                self.model.move_tabs(
                    &[tab],
                    Some(window),
                    end.saturating_sub(1),
                    Some(Some(g)),
                    now,
                )?;
                if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                    t.set_group_id(Some(g));
                }
            }
        }
        Ok(tab)
    }

    /// Whether a tool of `task` may act on `tab` (13 §7.3): its own agent
    /// tabs while the person is not looking at them; the person's tabs per
    /// `ai.agentsInUserTabs`, reads only for `readOnly`, never their
    /// active tab's address.
    pub(super) fn tool_may_use(
        &self,
        task: Option<TaskId>,
        tab: TabId,
        read_only: bool,
        navigates: bool,
    ) -> Result<(), MadoError> {
        let t = self
            .model
            .tabs
            .get(&tab)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
        let active = self
            .model
            .windows
            .get(&t.window_id)
            .is_some_and(|w| w.active_tab_id == Some(tab));
        if let Some(a) = &t.agent
            && Some(a.task_id) == task
        {
            if active && !read_only && !self.handed_back.contains(&tab) {
                return Err(MadoError::new(ErrorCode::Conflict, "error.ai.userTookOver"));
            }
            return Ok(());
        }
        let mode = self.setting_str("ai.agentsInUserTabs");
        if mode == "never" {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.userTabsNever",
            ));
        }
        if read_only {
            return Ok(());
        }
        if navigates && active {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.notYourActiveTab",
            ));
        }
        if mode == "readOnly" {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.userTabsReadOnly",
            ));
        }
        // `askFirst`: an approval card arrives with M5-T11; until then no.
        Err(MadoError::new(
            ErrorCode::PermissionDenied,
            "error.ai.needsApproval",
        ))
    }
}
