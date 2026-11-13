#![cfg(test)]

use super::CHECK_AND_INCREMENT_BY_N_SCRIPT;

#[test]
fn lua_script_reads_redis_server_time() {
    assert!(CHECK_AND_INCREMENT_BY_N_SCRIPT.contains("local time_reply = redis.call('TIME')"));
    assert!(CHECK_AND_INCREMENT_BY_N_SCRIPT.contains("local now = tonumber(time_reply[1])"));
}

#[test]
fn lua_script_checks_every_descriptor_before_writing_any_counter() {
    let script = CHECK_AND_INCREMENT_BY_N_SCRIPT;
    let validation = script
        .find("return { 1, i, current_counter, limit }")
        .expect("the over-limit reply");
    let delete = script.find("redis.call('DEL'").expect("the sibling reset");
    let set = script.find("redis.call('SET'").expect("the window reset");
    let increment = script
        .find("redis.call('INCRBY'")
        .expect("the counter increment");
    assert!(validation < delete);
    assert!(validation < set);
    assert!(validation < increment);
}

#[test]
fn lua_script_reads_four_arguments_for_each_counter() {
    let script = CHECK_AND_INCREMENT_BY_N_SCRIPT;
    assert!(script.contains("local descriptor_count = #KEYS / 2"));
    assert!(script.contains("local arg_base = (i - 1) * 4 + 1"));
    assert!(script.contains("ARGV[(i-1)*4 + 1] = limit"));
    assert!(script.contains("ARGV[(i-1)*4 + 2] = increment"));
    assert!(script.contains("ARGV[(i-1)*4 + 3] = ttl_seconds"));
    assert!(script.contains("ARGV[(i-1)*4 + 4] = window_size_seconds"));
}

#[test]
fn lua_script_returns_a_window_start_after_each_counter() {
    let script = CHECK_AND_INCREMENT_BY_N_SCRIPT;
    assert!(script.contains("local results = { 0 }"));
    assert!(script.contains("table.insert(results, increment)"));
    assert!(script.contains("table.insert(results, new_counter)"));
    assert!(script.contains("table.insert(results, active_window_start)"));
}

#[test]
fn lua_script_resets_sibling_counter_keys_and_keeps_their_ttls() {
    let script = CHECK_AND_INCREMENT_BY_N_SCRIPT;
    assert!(script.contains("redis.call('DEL', prefix .. ':requests', prefix .. ':tokens')"));
    assert!(script.contains("redis.call('EXPIRE', window_key, window_size)"));
    assert!(script.contains("if ttl > 0 then"));
    assert!(script.contains("redis.call('INCRBY', counter_key, increment)"));
    assert!(script.contains("if current_ttl == -1 and ttl > 0 then"));
}
