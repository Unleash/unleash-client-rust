// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/user_guide/activation_strategy>
use std::hash::BuildHasher;
use std::io::Cursor;
use std::net::IpAddr;
use std::{collections::hash_map::HashMap, str::FromStr};
use std::{collections::hash_set::HashSet, fmt::Display};

use chrono::{DateTime, Utc};
use ipnet::IpNet;
use log::{trace, warn};
use murmur3::murmur3_32;
use rand::Rng;
use semver::Version;

use crate::api::{Constraint, ConstraintExpression};
use crate::context::Context;

/// Memoise feature state for a strategy.
pub type Strategy =
    Box<dyn Fn(Option<HashMap<String, String>>) -> Evaluate + Sync + Send + 'static>;
/// Apply memoised state to a context.
pub trait Evaluator: Fn(&Context) -> bool {
    fn clone_boxed(&self) -> Box<dyn Evaluator + Send + Sync + 'static>;
}
pub type Evaluate = Box<dyn Evaluator + Send + Sync + 'static>;

impl<T> Evaluator for T
where
    T: 'static + Clone + Sync + Send + Fn(&Context) -> bool,
{
    fn clone_boxed(&self) -> Box<dyn Evaluator + Send + Sync + 'static> {
        Box::new(T::clone(self))
    }
}

impl Clone for Box<dyn Evaluator + Send + Sync + 'static> {
    fn clone(&self) -> Self {
        self.as_ref().clone_boxed()
    }
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#standard>
pub fn default<S: BuildHasher>(_: Option<HashMap<String, String, S>>) -> Evaluate {
    Box::new(|_: &Context| -> bool { true })
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#userids>
/// userIds: user,ids,to,match
pub fn user_with_id<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    let mut uids: HashSet<String> = HashSet::new();
    if let Some(parameters) = parameters {
        if let Some(uids_list) = parameters.get("userIds") {
            for uid in uids_list.split(',') {
                uids.insert(uid.trim().into());
            }
        }
    }
    Box::new(move |context: &Context| -> bool {
        context
            .user_id
            .as_ref()
            .map(|uid| uids.contains(uid))
            .unwrap_or(false)
    })
}

/// Get the group and rollout parameters for gradual rollouts.
///
/// When no group is supplied, group is set to "".
///
/// Checks the following parameter keys:
/// `groupId`: defines the hash group, used to either correlate or prevent correlation across toggles.
/// rollout_key: supplied by the caller, this keys value is the percent of the hashed results to enable.
pub fn group_and_rollout<S: BuildHasher>(
    parameters: &Option<HashMap<String, String, S>>,
    rollout_key: &str,
) -> (String, u32) {
    let parameters = if let Some(parameters) = parameters {
        parameters
    } else {
        return ("".into(), 0);
    };
    let group = if let Some(group) = parameters.get("groupId") {
        group.to_string()
    } else {
        "".into()
    };

    let mut rollout = 0;
    if let Some(rollout_str) = parameters.get(rollout_key) {
        if let Ok(percent) = rollout_str.parse::<u32>() {
            rollout = percent
        }
    }
    (group, rollout)
}

/// Implement partial rollout given a group a variable part and a rollout amount
pub fn partial_rollout(group: &str, variable: Option<&String>, rollout: u32) -> bool {
    let variable = if let Some(variable) = variable {
        variable
    } else {
        return false;
    };
    match rollout {
        // No need to hash when set to 0 or 100
        0 => false,
        100 => true,
        rollout => {
            if let Ok(normalised) = normalised_hash(group, variable, 100) {
                rollout >= normalised
            } else {
                false
            }
        }
    }
}

/// Calculates a hash in the standard way expected for Unleash clients. Not
/// required for extension strategies, but reusing this is probably a good idea
/// for consistency across implementations.
pub fn normalised_hash(group: &str, identifier: &str, modulus: u32) -> std::io::Result<u32> {
    normalised_hash_internal(group, identifier, modulus, 0)
}

const VARIANT_NORMALIZATION_SEED: u32 = 86028157;

/// Calculates a hash for **variant distribution** in the standard way
/// expected for Unleash clients. This differs from the
/// [`normalised_hash`] function in that it uses a different seed to
///  ensure a fair distribution.
pub fn normalised_variant_hash(
    group: &str,
    identifier: &str,
    modulus: u32,
) -> std::io::Result<u32> {
    normalised_hash_internal(group, identifier, modulus, VARIANT_NORMALIZATION_SEED)
}

fn normalised_hash_internal(
    group: &str,
    identifier: &str,
    modulus: u32,
    seed: u32,
) -> std::io::Result<u32> {
    // See https://github.com/stusmall/murmur3/pull/16 : .chain may avoid
    // copying in the general case, and may be faster (though perhaps
    // benchmarking would be useful - small datasizes here could make the best
    // path non-obvious) - but until murmur3 is fixed, we need to provide it
    // with a single string no matter what.
    let mut reader = Cursor::new(format!("{}:{}", &group, &identifier));
    murmur3_32(&mut reader, seed).map(|hash_result| hash_result % modulus + 1)
}

// Build a closure to handle session id rollouts, parameterised by groupId and a
// metaparameter of the percentage taken from rollout_key.
fn _session_id<S: BuildHasher>(
    parameters: Option<HashMap<String, String, S>>,
    rollout_key: &str,
) -> Evaluate {
    let (group, rollout) = group_and_rollout(&parameters, rollout_key);
    Box::new(move |context: &Context| -> bool {
        partial_rollout(&group, context.session_id.as_ref(), rollout)
    })
}

// Build a closure to handle user id rollouts, parameterised by groupId and a
// metaparameter of the percentage taken from rollout_key.
fn _user_id<S: BuildHasher>(
    parameters: Option<HashMap<String, String, S>>,
    rollout_key: &str,
) -> Evaluate {
    let (group, rollout) = group_and_rollout(&parameters, rollout_key);
    Box::new(move |context: &Context| -> bool {
        partial_rollout(&group, context.user_id.as_ref(), rollout)
    })
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#gradual-rollout>
/// stickiness: [default|userId|sessionId|random]
/// groupId: hash key
/// rollout: percentage
pub fn flexible_rollout<S: BuildHasher>(
    parameters: Option<HashMap<String, String, S>>,
) -> Evaluate {
    let unwrapped_parameters = if let Some(parameters) = &parameters {
        parameters
    } else {
        return Box::new(|_| false);
    };
    match if let Some(stickiness) = unwrapped_parameters.get("stickiness") {
        stickiness.as_str()
    } else {
        return Box::new(|_| false);
    } {
        "default" => {
            // user, session, random in that order.
            let (group, rollout) = group_and_rollout(&parameters, "rollout");
            Box::new(move |context: &Context| -> bool {
                if context.user_id.is_some() {
                    partial_rollout(&group, context.user_id.as_ref(), rollout)
                } else if context.session_id.is_some() {
                    partial_rollout(&group, context.session_id.as_ref(), rollout)
                } else {
                    pick_random(rollout as u8)
                }
            })
        }
        "userId" => _user_id(parameters, "rollout"),
        "sessionId" => _session_id(parameters, "rollout"),
        "random" => _random(parameters, "rollout"),
        _ => Box::new(|_| false),
    }
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#gradualrolloutuserid-deprecated-from-v4---use-gradual-rollout-instead>
/// percentage: 0-100
/// groupId: hash key
pub fn user_id<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _user_id(parameters, "percentage")
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#gradualrolloutsessionid-deprecated-from-v4---use-gradual-rollout-instead>
/// percentage: 0-100
/// groupId: hash key
pub fn session_id<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _session_id(parameters, "percentage")
}

/// Perform the is-enabled check for a random rollout of pct.
fn pick_random(pct: u8) -> bool {
    match pct {
        0 => false,
        100 => true,
        pct => {
            let mut rng = rand::rng();
            // generates 0's but not 100's.
            let picked = rng.random_range(0..100);
            pct > picked
        }
    }
}

// Build a closure to handle random rollouts, parameterised by a
// metaparameter of the percentage taken from rollout_key.
pub fn _random<S: BuildHasher>(
    parameters: Option<HashMap<String, String, S>>,
    rollout_key: &str,
) -> Evaluate {
    let mut pct = 0;
    if let Some(parameters) = parameters {
        if let Some(pct_str) = parameters.get(rollout_key) {
            if let Ok(percent) = pct_str.parse::<u8>() {
                pct = percent
            }
        }
    }
    Box::new(move |_: &Context| -> bool { pick_random(pct) })
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#gradualrolloutrandom-deprecated-from-v4---use-gradual-rollout-instead>
/// percentage: percentage 0-100
pub fn random<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _random(parameters, "percentage")
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#ips>
/// IPs: 1.2.3.4,AB::CD::::EF,1.2/8
pub fn remote_address<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    // TODO: this could be optimised given the inherent radix structure, but its
    // not exactly hot-path.
    let mut ips: Vec<IpNet> = Vec::new();

    if let Some(parameters) = parameters {
        if let Some(ips_str) = parameters.get("IPs") {
            for ip_str in ips_str.split(',') {
                let ip_parsed = _parse_ip(ip_str.trim());
                if let Ok(ip) = ip_parsed {
                    ips.push(ip)
                }
            }
        }
    }

    Box::new(move |context: &Context| -> bool {
        if let Some(remote_address) = &context.remote_address {
            for ip in &ips {
                if ip.contains(&remote_address.0) {
                    return true;
                }
            }
        }
        false
    })
}

/// <https://docs.getunleash.io/user_guide/activation_strategy#hostnames>
/// hostNames: names,of,hosts
pub fn hostname<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    let mut result = false;
    hostname::get().ok().and_then(|this_hostname| {
        parameters.map(|parameters| {
            parameters.get("hostNames").map(|hostnames: &String| {
                for hostname in hostnames.split(',') {
                    if this_hostname == hostname.trim() {
                        result = true;
                    }
                }
                false
            })
        })
    });

    Box::new(move |_: &Context| -> bool { result })
}

fn lower_case_if<S: Display>(case_insensitive: bool) -> impl Fn(S) -> String {
    move |s| {
        if case_insensitive {
            s.to_string().to_lowercase()
        } else {
            s.to_string()
        }
    }
}

fn handle_parsable_op<T, C, F>(getter: F, compare_fn: C) -> Evaluate
where
    T: FromStr,
    C: Fn(T) -> bool + Clone + Sync + Send + 'static,
    F: Fn(&Context) -> Option<&String> + Clone + Sync + Send + 'static,
{
    Box::new(move |context: &Context| {
        getter(context)
            .and_then(|v| v.parse::<T>().ok())
            .map(&compare_fn)
            .unwrap_or(false)
    })
}

fn handle_str_op<T, C, F>(
    values: Vec<String>,
    getter: F,
    case_insensitive: bool,
    compare_fn: C,
) -> Evaluate
where
    T: Display,
    C: Fn(&String, &String) -> bool + Clone + Sync + Send + 'static,
    F: Fn(&Context) -> Option<&T> + Clone + Sync + Send + 'static,
{
    let as_vec: Vec<String> = values.iter().map(lower_case_if(case_insensitive)).collect();
    Box::new(move |context: &Context| {
        getter(context)
            .map(lower_case_if(case_insensitive))
            .map(|v| as_vec.iter().any(|entry| compare_fn(&v, entry)))
            .unwrap_or(false)
    })
}

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_string<F, B>(
    expression: ConstraintExpression,
    apply_invert: B,
    case_insensitive: bool,
    getter: F,
) -> Evaluate
where
    F: Fn(&Context) -> Option<&String> + Clone + Sync + Send + 'static,
    B: Fn(bool) -> bool + Sync + Send + Clone + 'static,
{
    let compiled_fn: Box<dyn Evaluator + Send + Sync + 'static> = match expression {
        ConstraintExpression::In { values } => {
            let as_set: HashSet<String> = values.iter().cloned().collect();

            Box::new(move |context: &Context| {
                getter(context).map(|v| as_set.contains(v)).unwrap_or(false)
            })
        }
        ConstraintExpression::NotIn { values } => {
            if values.is_empty() {
                Box::new(|_| true)
            } else {
                let as_set: HashSet<String> = values.iter().cloned().collect();
                Box::new(move |context: &Context| {
                    getter(context).map(|v| !as_set.contains(v)).unwrap_or(true)
                })
            }
        }
        ConstraintExpression::StrContains { values } => {
            handle_str_op(values, getter, case_insensitive, |v, entry| {
                v.contains(entry)
            })
        }
        ConstraintExpression::StrStartsWith { values } => {
            handle_str_op(values, getter, case_insensitive, |v, entry| {
                v.starts_with(entry)
            })
        }
        ConstraintExpression::StrEndsWith { values } => {
            handle_str_op(values, getter, case_insensitive, |v, entry| {
                v.ends_with(entry)
            })
        }
        ConstraintExpression::NumEq { value } => {
            handle_parsable_op(getter, move |v: f64| v == value)
        }
        ConstraintExpression::NumGT { value } => {
            handle_parsable_op(getter, move |v: f64| v > value)
        }
        ConstraintExpression::NumGTE { value } => {
            handle_parsable_op(getter, move |v: f64| v >= value)
        }
        ConstraintExpression::NumLT { value } => {
            handle_parsable_op(getter, move |v: f64| v < value)
        }
        ConstraintExpression::NumLTE { value } => {
            handle_parsable_op(getter, move |v: f64| v <= value)
        }
        ConstraintExpression::SemverEq { value } => {
            handle_parsable_op(getter, move |v: Version| v == value)
        }
        ConstraintExpression::SemverGT { value } => {
            handle_parsable_op(getter, move |v: Version| v > value)
        }
        ConstraintExpression::SemverLT { value } => {
            handle_parsable_op(getter, move |v: Version| v < value)
        }
        _ => Box::new(|_| false),
    };

    Box::new(move |context: &Context| apply_invert(compiled_fn(context)))
}

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_date<F, B>(
    expression: ConstraintExpression,
    apply_invert: B,
    getter: F,
) -> Evaluate
where
    F: Fn(&Context) -> Option<&DateTime<Utc>> + Clone + Sync + Send + 'static,
    B: Fn(bool) -> bool + Sync + Send + Clone + 'static,
{
    let compiled_fn: Box<dyn Evaluator + Send + Sync + 'static> = match expression {
        ConstraintExpression::DateAfter { value } => {
            Box::new(move |context: &Context| getter(context).map(|v| *v > value).unwrap_or(false))
        }
        ConstraintExpression::DateBefore { value } => {
            Box::new(move |context: &Context| getter(context).map(|v| *v < value).unwrap_or(false))
        }
        _ => Box::new(|_| false),
    };
    Box::new(move |context: &Context| apply_invert(compiled_fn(context)))
}

fn _ip_to_vec(ips: &[String]) -> Vec<IpNet> {
    let mut result = Vec::new();
    for ip_str in ips {
        let ip_parsed = _parse_ip(ip_str.trim());
        if let Ok(ip) = ip_parsed {
            result.push(ip);
        } else {
            warn!("Could not parse IP address {ip_str:?}");
        }
    }
    result
}

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_host<F, B>(
    expression: ConstraintExpression,
    apply_invert: B,
    case_insensitive: bool,
    getter: F,
) -> Evaluate
where
    F: Fn(&Context) -> Option<&crate::context::IPAddress> + Clone + Sync + Send + 'static,
    B: Fn(bool) -> bool + Sync + Send + Clone + 'static,
{
    let compiled_fn: Box<dyn Evaluator + Send + Sync + 'static> = match expression {
        ConstraintExpression::In { values } => {
            let ips = _ip_to_vec(&values[..]);
            Box::new(move |context: &Context| {
                getter(context)
                    .map(|remote_address| {
                        for ip in &ips {
                            if ip.contains(&remote_address.0) {
                                return true;
                            }
                        }
                        false
                    })
                    .unwrap_or(false)
            })
        }
        ConstraintExpression::NotIn { values } => {
            if values.is_empty() {
                Box::new(|_| false)
            } else {
                let ips = _ip_to_vec(&values[..]);
                Box::new(move |context: &Context| {
                    getter(context)
                        .map(|remote_address| {
                            if ips.is_empty() {
                                return false;
                            }
                            for ip in &ips {
                                if ip.contains(&remote_address.0) {
                                    return false;
                                }
                            }
                            true
                        })
                        .unwrap_or(true)
                })
            }
        }
        ConstraintExpression::StrContains { values } => handle_str_op(
            values,
            move |ctx: &Context| getter(ctx).map(|v| &v.0),
            case_insensitive,
            |v, entry| v.contains(entry),
        ),
        ConstraintExpression::StrStartsWith { values } => handle_str_op(
            values,
            move |ctx: &Context| getter(ctx).map(|v| &v.0),
            case_insensitive,
            |v, entry| v.starts_with(entry),
        ),
        ConstraintExpression::StrEndsWith { values } => handle_str_op(
            values,
            move |ctx: &Context| getter(ctx).map(|v| &v.0),
            case_insensitive,
            |v, entry| v.ends_with(entry),
        ),
        _ => Box::new(|_| false),
    };
    Box::new(move |context: &Context| apply_invert(compiled_fn(context)))
}

fn _apply_invert(inverted: bool) -> impl Fn(bool) -> bool + Clone {
    move |state| {
        if inverted {
            !state
        } else {
            state
        }
    }
}

fn _compile_constraints(constraints: Vec<Constraint>) -> Vec<Evaluate> {
    constraints
        .into_iter()
        .map(|constraint| {
            let (context_name, expression, inverted, case_insensitive) = (
                constraint.context_name,
                constraint.expression,
                constraint.inverted,
                constraint.case_insensitive,
            );
            let apply_invert = _apply_invert(inverted);

            match context_name.as_str() {
                "appName" => _compile_constraint_string(
                    expression,
                    apply_invert,
                    case_insensitive,
                    |context| Some(&context.app_name),
                ),
                "environment" => _compile_constraint_string(
                    expression,
                    apply_invert,
                    case_insensitive,
                    |context| Some(&context.environment),
                ),
                "remoteAddress" => _compile_constraint_host(
                    expression,
                    apply_invert,
                    case_insensitive,
                    |context| context.remote_address.as_ref(),
                ),
                "sessionId" => _compile_constraint_string(
                    expression,
                    apply_invert,
                    case_insensitive,
                    |context| context.session_id.as_ref(),
                ),
                "userId" => _compile_constraint_string(
                    expression,
                    apply_invert,
                    case_insensitive,
                    |context| context.user_id.as_ref(),
                ),
                "currentTime" => _compile_constraint_date(expression, apply_invert, |context| {
                    context.current_time.as_ref()
                }),
                _ => _compile_constraint_string(
                    expression,
                    apply_invert,
                    case_insensitive,
                    move |context| context.properties.get(&context_name),
                ),
            }
        })
        .collect()
}

/// This function is a strategy decorator which compiles to nothing when
/// there are no constraints, or to a constraint evaluating test if there are.
pub fn constrain<S: Fn(Option<HashMap<String, String>>) -> Evaluate + Sync + Send + 'static>(
    constraints: Option<Vec<Constraint>>,
    strategy: &S,
    parameters: Option<HashMap<String, String>>,
) -> Evaluate {
    let compiled_strategy = strategy(parameters);
    match constraints {
        None => {
            trace!("constrain: no constraints, bypassing");
            compiled_strategy
        }
        Some(constraints) => {
            if constraints.is_empty() {
                trace!("constrain: empty constraints list, bypassing");
                compiled_strategy
            } else {
                trace!("constrain: compiling constraints list {constraints:?}");
                let constraints = _compile_constraints(constraints);
                // Create a closure that will evaluate against the context.
                Box::new(move |context| {
                    // Check every constraint; if all match, permit
                    for constraint in &constraints {
                        if !constraint(context) {
                            return false;
                        }
                    }
                    compiled_strategy(context)
                })
            }
        }
    }
}

fn _parse_ip(ip: &str) -> Result<IpNet, std::net::AddrParseError> {
    ip.parse::<IpNet>()
        .or_else(|_| ip.parse::<IpAddr>().map(|addr| addr.into()))
}

#[cfg(test)]
mod tests {
    use std::default::Default;
    use std::{collections::hash_map::HashMap, str::FromStr};

    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
    use maplit::hashmap;
    use semver::Version;

    use crate::api::{Constraint, ConstraintExpression};
    use crate::context::{Context, IPAddress};

    fn parse_ip(addr: &str) -> Option<IPAddress> {
        Some(IPAddress(addr.parse().unwrap()))
    }

    fn default_constraint() -> Constraint {
        Constraint {
            context_name: "".into(),
            case_insensitive: false,
            inverted: false,
            expression: ConstraintExpression::In { values: vec![] },
        }
    }

    #[test]
    fn test_constrain_general() {
        // Without constraints, things should just pass through
        let context = Context::default();
        assert!(super::constrain(None, &super::default, None)(&context));

        // An empty constraint list acts like a missing one
        let context = Context::default();
        assert!(super::constrain(Some(vec![]), &super::default, None)(
            &context
        ));
    }

    #[test]
    fn test_constrain_with_in_constraints() {
        // An empty constraint gets disabled
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "".into(),
                expression: ConstraintExpression::In { values: vec![] },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // A missing field in context for NotIn delegates
        let context = Context::default();
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "customFieldMissing".into(),
                expression: ConstraintExpression::NotIn {
                    values: vec!["s1".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // A mismatched constraint acts like an empty constraint
        let context = Context {
            environment: "production".into(),
            ..Default::default()
        };
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::In {
                    values: vec!["development".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // a matched Not In acts like an empty constraint
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::NotIn {
                    values: vec!["development".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // a matched In in either first or second (etc) places delegates
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::In {
                    values: vec!["development".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::In {
                    values: vec!["development".into()]
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // second place
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::In {
                    values: vec!["staging".into(), "development".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // a not matched Not In across 1st and second etc delegates
        let context = Context {
            environment: "production".into(),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::NotIn {
                    values: vec!["staging".into(), "development".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::NotIn {
                    values: vec!["staging".into(), "development".into()]
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // Context keys can be chosen by the context_name field:
        // .environment is used above.
        // .user_id
        let context = Context {
            user_id: Some("fred".into()),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "userId".into(),
                expression: ConstraintExpression::In {
                    values: vec!["fred".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // .session_id
        let context = Context {
            session_id: Some("qwerty".into()),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "sessionId".into(),
                expression: ConstraintExpression::In {
                    values: vec!["qwerty".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // .remote_address
        let context = Context {
            remote_address: parse_ip("10.20.30.40"),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "remoteAddress".into(),
                expression: ConstraintExpression::In {
                    values: vec!["10.0.0.0/8".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));
        let context = Context {
            remote_address: parse_ip("1.2.3.4"),
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "remoteAddress".into(),
                expression: ConstraintExpression::NotIn {
                    values: vec!["10.0.0.0/8".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // multiple constraints are ANDed together
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        // true ^ true => true
        assert!(super::constrain(
            Some(vec![
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()]
                    },
                    ..default_constraint()
                },
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()]
                    },
                    ..default_constraint()
                },
            ]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()]
                    },
                    ..default_constraint()
                },
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In { values: vec![] },
                    ..default_constraint()
                }
            ]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_constrain_with_date_constraints() {
        let now = Utc::now();
        let context = Context {
            current_time: Some(now),
            ..Default::default()
        };
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateBefore {
                    value: Utc::now() - TimeDelta::seconds(30)
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateAfter {
                    value: Utc::now() - TimeDelta::seconds(30)
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateAfter {
                    value: Utc::now() - TimeDelta::seconds(30)
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        let context = Context {
            current_time: DateTime::<FixedOffset>::parse_from_rfc3339("2024-07-18T17:18:25.844Z")
                .ok()
                .map(|date| date.to_utc()),
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateBefore { value: Utc::now() },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateBefore { value: Utc::now() },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                expression: ConstraintExpression::DateAfter { value: Utc::now() },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // date comparison only works for currentTime
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                expression: ConstraintExpression::DateBefore { value: Utc::now() },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_constrain_with_semver_constraints() {
        let context = Context {
            properties: hashmap! {
                "version".into() => "1.2.3-rc.2".into()
            },
            ..Default::default()
        };
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                expression: ConstraintExpression::SemverLT {
                    value: Version::from_str("1.2.3").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                expression: ConstraintExpression::SemverGT {
                    value: Version::from_str("1.2.2").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                expression: ConstraintExpression::SemverEq {
                    value: Version::from_str("1.2.3-rc.2").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        let context = Context {
            properties: hashmap! {
                "app_version".into() => "1.0.0-alpha.1".into()
            },
            ..Default::default()
        };

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverLT {
                    value: Version::from_str("0.155.0").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverLT {
                    value: Version::from_str("0.155.0").unwrap()
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverGT {
                    value: Version::from_str("1.0.0-beta.1").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverGT {
                    value: Version::from_str("1.0.0-beta.1").unwrap()
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverEq {
                    value: Version::from_str("1.0.0-beta.10").unwrap()
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "app_version".into(),
                expression: ConstraintExpression::SemverEq {
                    value: Version::from_str("1.0.0-beta.10").unwrap()
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_constrain_with_str_constraints() {
        let context = Context {
            app_name: "gondola".into(),
            ..Default::default()
        };

        // contains
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrContains {
                    values: vec!["ondo".into(), "gigabad".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrContains {
                    values: vec!["Ondo".into(), "gigabad".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrContains {
                    values: vec!["Ondo".into(), "gigabad".into()]
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // case insensitive
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrContains {
                    values: vec!["Ondo".into(), "gigabad".into()]
                },
                case_insensitive: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["and".into(), "gon".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["and".into(), "Gon".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["and".into(), "Gon".into()]
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // case insensitive
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["and".into(), "Gon".into()]
                },
                case_insensitive: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["ola".into(), "oga".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["Ola".into(), "Oga".into()]
                },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["Ola".into(), "Oga".into()]
                },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // case insensitive
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "appName".into(),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["Ola".into(), "Oga".into()]
                },
                case_insensitive: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_constrain_with_num_constraints() {
        let context = Context {
            properties: hashmap! {
                "times".into() => "30".into()
            },
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumEq { value: 30.0 },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumLT { value: 31.0 },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumLTE { value: 40.0 },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumGT { value: 29.0 },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumGTE { value: 30.0 },
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        // inverted
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumEq { value: 30.0 },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumLT { value: 31.0 },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumLTE { value: 40.0 },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumGT { value: 29.0 },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "times".into(),
                expression: ConstraintExpression::NumGTE { value: 30.0 },
                inverted: true,
                ..default_constraint()
            }]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_user_with_id() {
        let params: HashMap<String, String> = hashmap! {
            "userIds".into() => "fred,barney".into(),
        };
        assert!(super::user_with_id(Some(params.clone()))(&Context {
            user_id: Some("fred".into()),
            ..Default::default()
        }));
        assert!(super::user_with_id(Some(params.clone()))(&Context {
            user_id: Some("barney".into()),
            ..Default::default()
        }));
        assert!(!super::user_with_id(Some(params))(&Context {
            user_id: Some("betty".into()),
            ..Default::default()
        }));
    }

    #[test]
    fn test_flexible_rollout() {
        // random
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "random".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Default::default();
        assert!(!super::flexible_rollout(Some(params))(&c));

        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "random".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Default::default();
        assert!(super::flexible_rollout(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "random".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Default::default();
        assert!(!super::flexible_rollout(Some(params))(&c));

        // Could parameterise this by SESSION and USER, but its barely long
        // enough to bother and the explicitness in failures has merit.
        // sessionId
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "sessionId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "sessionId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params))(&c));
        // Check rollout works
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "sessionId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            session_id: Some("session2".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params))(&c));
        // Check groupId modifies the hash order
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "sessionId".into(),
            "groupId".into() => "group3".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            session_id: Some("session2".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params))(&c));

        // userId
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "userId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "userId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params))(&c));
        // Check rollout works
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "userId".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            user_id: Some("user3".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params))(&c));
        // Check groupId modifies the hash order
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "userId".into(),
            "groupId".into() => "group2".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            user_id: Some("user3".into()),
            ..Default::default()
        };
        assert!(!super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert!(super::flexible_rollout(Some(params))(&c));
    }

    #[test]
    fn test_random() {
        let params: HashMap<String, String> = hashmap! {
            "percentage".into() => "0".into()
        };
        let c: Context = Default::default();
        assert!(!super::random(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "percentage".into() => "100".into()
        };
        let c: Context = Default::default();
        assert!(super::random(Some(params))(&c));
    }

    #[test]
    fn test_remote_address() {
        let params: HashMap<String, String> = hashmap! {
            "IPs".into() => "1.2.0.0/8,2.3.4.5,2222:FF:0:1234::/64".into()
        };
        let c: Context = Context {
            remote_address: parse_ip("1.2.3.4"),
            ..Default::default()
        };
        assert!(super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: parse_ip("2.3.4.5"),
            ..Default::default()
        };
        assert!(super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: parse_ip("2222:FF:0:1234::FDEC"),
            ..Default::default()
        };
        assert!(super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: parse_ip("2.3.4.4"),
            ..Default::default()
        };
        assert!(!super::remote_address(Some(params))(&c));
    }

    #[test]
    fn test_hostname() {
        let c: Context = Default::default();
        let this_hostname = hostname::get().unwrap().into_string().unwrap();
        let params: HashMap<String, String> = hashmap! {
            "hostNames".into() => format!("foo,{},bar", this_hostname)
        };
        assert!(super::hostname(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "hostNames".into() => "foo,bar".into()
        };
        assert!(!super::hostname(Some(params))(&c));
    }

    #[test]
    fn normalised_hash() {
        assert!(50 > super::normalised_hash("AB12A", "122", 100).unwrap());
    }

    #[test]
    fn test_normalized_hash() {
        assert_eq!(73, super::normalised_hash("gr1", "123", 100).unwrap());
        assert_eq!(25, super::normalised_hash("groupX", "999", 100).unwrap());
    }

    #[test]
    fn test_normalised_variant_hash() {
        assert_eq!(
            96,
            super::normalised_variant_hash("gr1", "123", 100).unwrap()
        );
        assert_eq!(
            60,
            super::normalised_variant_hash("groupX", "999", 100).unwrap()
        );
    }
}
