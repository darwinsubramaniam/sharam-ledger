/// Embedded schema files. `apply_control_schema` and `apply_tenant_schema`
/// send these as multi-statement queries.
pub const CONTROL_SCHEMA: &str = include_str!("../schema/control/000_init.surql");
pub const TENANT_SCHEMA: &str = include_str!("../schema/tenant/000_init.surql");
