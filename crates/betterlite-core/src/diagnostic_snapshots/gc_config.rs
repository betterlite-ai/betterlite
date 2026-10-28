use betterlite_config::loader::EnvironmentLookup;

pub fn gc_thresholds<L>(config: &L) -> Option<[u64; 3]>
where
    L: EnvironmentLookup,
{
    let value = config.lookup("PYTHON_GC_THRESHOLD")?;
    let mut parts = value.split(',').map(str::trim).map(str::parse::<u64>);
    let thresholds = [
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ];
    if parts.next().is_some() {
        return None;
    }
    Some(thresholds)
}
