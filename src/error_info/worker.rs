use crate::error_info::ErrorInfo;

pub const NOT_AUTHORIZED: ErrorInfo =
    ErrorInfo::Forbidden("worker.not_authorized", "无权操作此 Worker");
pub const SCHEDULE_NOT_YOURS: ErrorInfo =
    ErrorInfo::Forbidden("worker_schedule.not_yours", "无权操作他人的定时任务");
pub const EXECUTION_NOT_YOURS: ErrorInfo =
    ErrorInfo::Forbidden("worker_execution.not_yours", "无权查看他人的执行记录");
