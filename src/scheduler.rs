use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{Datelike, Local, Timelike};

use crate::app::{app_state, current_timestamp};
use crate::{save_app_config, AppConfig, ScheduledTaskConfig};

#[derive(Copy, Clone)]
enum TaskKind {
    UpdateSubscriptions,
    UpdateGeoip,
}

fn task_name(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::UpdateSubscriptions => "subscription_auto_update",
        TaskKind::UpdateGeoip => "geoip_auto_update",
    }
}

fn task_config<'a>(
    config: &'a AppConfig,
    kind: TaskKind,
) -> Option<&'a ScheduledTaskConfig> {
    match kind {
        TaskKind::UpdateSubscriptions => config.subscription_auto_update.as_ref(),
        TaskKind::UpdateGeoip => config.geoip_auto_update.as_ref(),
    }
}

fn task_config_mut<'a>(
    config: &'a mut AppConfig,
    kind: TaskKind,
) -> &'a mut ScheduledTaskConfig {
    match kind {
        TaskKind::UpdateSubscriptions => {
            config
                .subscription_auto_update
                .get_or_insert_with(ScheduledTaskConfig::default)
        }
        TaskKind::UpdateGeoip => config
            .geoip_auto_update
            .get_or_insert_with(ScheduledTaskConfig::default),
    }
}

static SUBS_RUNNING: AtomicBool = AtomicBool::new(false);
static GEOIP_RUNNING: AtomicBool = AtomicBool::new(false);

fn task_flag(kind: TaskKind) -> &'static AtomicBool {
    match kind {
        TaskKind::UpdateSubscriptions => &SUBS_RUNNING,
        TaskKind::UpdateGeoip => &GEOIP_RUNNING,
    }
}

struct TaskLockGuard<'a> {
    flag: &'a AtomicBool,
    acquired: bool,
}

impl<'a> TaskLockGuard<'a> {
    fn lock(flag: &'a AtomicBool) -> Self {
        let acquired = flag
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok();
        TaskLockGuard { flag, acquired }
    }

    fn is_acquired(&self) -> bool {
        self.acquired
    }
}

impl<'a> Drop for TaskLockGuard<'a> {
    fn drop(&mut self) {
        if self.acquired {
            self.flag.store(false, Ordering::Release);
        }
    }
}

struct CronField {
    min: u32,
    max: u32,
    allowed: Vec<bool>,
}

impl CronField {
    fn new(min: u32, max: u32) -> Self {
        let len = (max - min + 1) as usize;
        CronField {
            min,
            max,
            allowed: vec![false; len],
        }
    }

    fn set(&mut self, value: u32) {
        if value < self.min || value > self.max {
            return;
        }
        let idx = (value - self.min) as usize;
        self.allowed[idx] = true;
    }

    fn set_range_step(&mut self, start: u32, end: u32, step: u32) {
        let step = step.max(1);
        let start = start.max(self.min);
        let end = end.min(self.max);
        let mut v = start;
        while v <= end {
            self.set(v);
            if v == u32::MAX {
                break;
            }
            v = v.saturating_add(step);
        }
    }

    fn set_all(&mut self) {
        for v in self.min..=self.max {
            self.set(v);
        }
    }

    fn matches(&self, value: u32) -> bool {
        if value < self.min || value > self.max {
            return false;
        }
        let idx = (value - self.min) as usize;
        self.allowed[idx]
    }
}

struct CronSchedule {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

impl CronSchedule {
    fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("cron expression must have 5 fields".to_string());
        }

        let minute = parse_field(parts[0], 0, 59)?;
        let hour = parse_field(parts[1], 0, 23)?;
        let day_of_month = parse_field(parts[2], 1, 31)?;
        let month = parse_field(parts[3], 1, 12)?;
        let day_of_week = parse_dow_field(parts[4])?;

        Ok(CronSchedule {
            minute,
            hour,
            day_of_month,
            month,
            day_of_week,
        })
    }

    fn next_after(&self, now: chrono::DateTime<Local>) -> Option<chrono::DateTime<Local>> {
        let mut candidate = now + chrono::Duration::minutes(1);
        // 最多向前搜索一年，避免死循环。
        let max_steps = 365 * 24 * 60;

        for _ in 0..max_steps {
            let minute = candidate.minute() as u32;
            let hour = candidate.hour() as u32;
            let day = candidate.day() as u32;
            let month = candidate.month() as u32;
            let dow = candidate.weekday().num_days_from_sunday() as u32;

            if self.minute.matches(minute)
                && self.hour.matches(hour)
                && self.day_of_month.matches(day)
                && self.month.matches(month)
                && self.day_of_week.matches(dow)
            {
                return Some(candidate);
            }

            candidate = candidate + chrono::Duration::minutes(1);
        }

        None
    }
}

fn parse_field(spec: &str, min: u32, max: u32) -> Result<CronField, String> {
    let mut field = CronField::new(min, max);

    let spec = spec.trim();
    if spec == "*" {
        field.set_all();
        return Ok(field);
    }

    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (range_part, step_part) = match part.split_once('/') {
            Some((r, s)) => (r.trim(), Some(s.trim())),
            None => (part, None),
        };

        let step = if let Some(s) = step_part {
            s.parse::<u32>()
                .map_err(|err| format!("invalid step value '{s}' in cron field '{spec}': {err}"))?
        } else {
            1
        };

        if range_part == "*" {
            field.set_range_step(min, max, step);
            continue;
        }

        let (start, end) = if let Some((a, b)) = range_part.split_once('-') {
            let start = a
                .trim()
                .parse::<u32>()
                .map_err(|err| format!("invalid range start '{a}' in cron field '{spec}': {err}"))?;
            let end = b
                .trim()
                .parse::<u32>()
                .map_err(|err| format!("invalid range end '{b}' in cron field '{spec}': {err}"))?;
            (start, end)
        } else {
            let value = range_part.parse::<u32>().map_err(|err| {
                format!("invalid value '{range_part}' in cron field '{spec}': {err}")
            })?;
            (value, value)
        };

        if start > end {
            return Err(format!(
                "invalid range '{start}-{end}' in cron field '{spec}'"
            ));
        }

        field.set_range_step(start, end, step);
    }

    Ok(field)
}

fn parse_dow_field(spec: &str) -> Result<CronField, String> {
    let mut field = parse_field(spec, 0, 7)?;

    // 将 7 视为星期日（0）
    if field.allowed.len() == 8 && field.allowed[7] {
        field.allowed[0] = true;
        field.allowed[7] = false;
    }

    Ok(field)
}

enum TaskRunState {
    Success,
    Skipped(String),
    Failed(String),
}

async fn execute_task(kind: TaskKind) -> TaskRunState {
    let name = task_name(kind);
    let flag = task_flag(kind);
    let guard = TaskLockGuard::lock(flag);

    if !guard.is_acquired() {
        return TaskRunState::Skipped("task already running".to_string());
    }

    let result = match kind {
        TaskKind::UpdateSubscriptions => crate::subscriptions::auto_update_subscriptions().await,
        TaskKind::UpdateGeoip => crate::geoip::update_geoip_db().await,
    };

    match result {
        Ok(()) => {
            tracing::info!("scheduler task '{name}' finished successfully");
            TaskRunState::Success
        }
        Err(err) => {
            if let Some(stripped) = err.strip_prefix("skipped:") {
                let msg = stripped.trim().to_string();
                tracing::info!("scheduler task '{name}' skipped: {msg}");
                TaskRunState::Skipped(msg)
            } else {
                tracing::error!("scheduler task '{name}' failed: {err}");
                TaskRunState::Failed(err)
            }
        }
    }
}

fn record_run_state(kind: TaskKind, state: &TaskRunState) {
    let name = task_name(kind);
    let app = app_state();

    let mut guard = app
        .app_config
        .write()
        .expect("app config rwlock poisoned");
    let config: &mut AppConfig = &mut guard;

    let task_cfg = task_config_mut(config, kind);
    task_cfg.last_run_time = Some(current_timestamp());

    match state {
        TaskRunState::Success => {
            task_cfg.last_run_status = Some("ok".to_string());
            task_cfg.last_run_message = None;
        }
        TaskRunState::Skipped(msg) => {
            task_cfg.last_run_status = Some("skipped".to_string());
            task_cfg.last_run_message = Some(msg.clone());
        }
        TaskRunState::Failed(msg) => {
            task_cfg.last_run_status = Some("error".to_string());
            task_cfg.last_run_message = Some(msg.clone());
        }
    }

    if let Err(err) = save_app_config(&app.data_root, config) {
        tracing::error!("scheduler[{name}] failed to save app config after run: {err}");
    }
}

async fn run_task_loop(kind: TaskKind) {
    let name = task_name(kind);

    loop {
        let task_cfg_opt = {
            let app = app_state();
            let guard = app
                .app_config
                .read()
                .expect("app config rwlock poisoned");
            let config: &AppConfig = &guard;
            task_config(config, kind).cloned()
        };

        let Some(task_cfg) = task_cfg_opt else {
            // 未配置任务：定期重试读取配置。
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        };

        if !task_cfg.enabled {
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let cron_expr = task_cfg.cron.trim().to_string();
        if cron_expr.is_empty() {
            tracing::warn!("scheduler[{name}] cron expression is empty");
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let schedule = match CronSchedule::parse(&cron_expr) {
            Ok(s) => s,
            Err(err) => {
                tracing::error!(
                    "scheduler[{name}] invalid cron expression '{cron_expr}': {err}"
                );
                record_run_state(
                    kind,
                    &TaskRunState::Failed(format!("invalid cron expression: {err}")),
                );
                tokio::time::sleep(Duration::from_secs(300)).await;
                continue;
            }
        };

        let now = Local::now();
        let next = match schedule.next_after(now) {
            Some(t) => t,
            None => {
                tracing::error!(
                    "scheduler[{name}] failed to compute next run time for cron '{cron_expr}'"
                );
                record_run_state(
                    kind,
                    &TaskRunState::Failed(
                        "failed to compute next run time from cron expression".to_string(),
                    ),
                );
                tokio::time::sleep(Duration::from_secs(300)).await;
                continue;
            }
        };

        let sleep_duration = match (next - now).to_std() {
            Ok(dur) if dur >= Duration::from_secs(1) => dur,
            _ => Duration::from_secs(60),
        };

        tracing::info!(
            "scheduler[{name}] next run at {} (in {:?})",
            next,
            sleep_duration
        );

        tokio::time::sleep(sleep_duration).await;

        let state = execute_task(kind).await;
        record_run_state(kind, &state);
    }
}

/// 启动所有后台定时任务调度循环。
pub fn start_scheduler() {
    tokio::spawn(run_task_loop(TaskKind::UpdateSubscriptions));
    tokio::spawn(run_task_loop(TaskKind::UpdateGeoip));
}
