// Copyright 2020 Cognite AS
//! <https://docs.getunleash.io/user_guide/activation_strategy>
use chrono::DateTime;
use chrono::FixedOffset;
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::hash::BuildHasher;
use std::io::Cursor;
use std::net::IpAddr;
use std::str::FromStr;

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
    if let Ok(normalised) = normalised_hash(group, variable, 100) {
        rollout > normalised
    } else {
        false
    }
}

/// Calculates a hash in the standard way expected for Unleash clients. Not
/// required for extension strategies, but reusing this is probably a good idea
/// for consistency across implementations.
pub fn normalised_hash(group: &str, identifier: &str, modulus: u32) -> std::io::Result<u32> {
    // See https://github.com/stusmall/murmur3/pull/16 : .chain may avoid
    // copying in the general case, and may be faster (though perhaps
    // benchmarking would be useful - small datasizes here could make the best
    // path non-obvious) - but until murmur3 is fixed, we need to provide it
    // with a single string no matter what.
    let mut reader = Cursor::new(format!("{}:{}", &group, &identifier));
    murmur3_32(&mut reader, 0).map(|hash_result| hash_result % modulus)
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
                    let picked = rand::thread_rng().gen_range(0..100);
                    rollout > picked
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
    Box::new(move |_: &Context| -> bool {
        let mut rng = rand::thread_rng();
        let picked = rng.gen_range(0..100);
        pct > picked
    })
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

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_string<F>(
    expression: ConstraintExpression,
    inverted: Option<bool>,
    getter: F,
) -> Evaluate
where
    F: Fn(&Context) -> Option<&String> + Clone + Sync + Send + 'static,
{
    match &expression {
        ConstraintExpression::In { values } => {
            let as_set: HashSet<String> = values.iter().cloned().collect();
            Box::new(move |context: &Context| {
                let value = getter(context).map(|v| as_set.contains(v)).unwrap_or(false);
                _handle_inversion(&value, &inverted)
            })
        }
        ConstraintExpression::NotIn { values } => {
            if values.is_empty() {
                Box::new(move |_| _handle_inversion(&false, &inverted))
            } else {
                let as_set: HashSet<String> = values.iter().cloned().collect();
                Box::new(move |context: &Context| {
                    let value = getter(context)
                        .map(|v| !as_set.contains(v))
                        .unwrap_or(false);
                    _handle_inversion(&value, &inverted)
                })
            }
        }
        ConstraintExpression::StrStartsWith {
            values,
            case_insensitive,
        } => {
            if values.is_empty() {
                Box::new(move |_| _handle_inversion(&false, &inverted))
            } else {
                let as_vec: Vec<String> = values.to_vec();
                Box::new(move |context: &Context| {
                    let value = getter(context)
                        .map(|v| as_vec.iter().any(|x| v.starts_with(x)))
                        .unwrap_or(false);
                    _handle_inversion(&value, &inverted)
                })
            }
        }
        ConstraintExpression::StrEndsWith {
            values,
            case_insensitive,
        } => {
            if values.is_empty() {
                Box::new(move |_| _handle_inversion(&false, &inverted))
            } else {
                let mut as_vec: Vec<String> = values.to_vec();
                let case_insensitive = case_insensitive.unwrap_or(false);
                if case_insensitive {
                    as_vec = as_vec.iter_mut().map(|x| x.to_lowercase()).collect();
                };
                Box::new(move |context: &Context| {
                    let value = getter(context)
                        .map(|v| {
                            as_vec.iter().any(|x| {
                                if case_insensitive {
                                    v.to_lowercase().ends_with(x)
                                } else {
                                    v.ends_with(x)
                                }
                            })
                        })
                        .unwrap_or(false);
                    _handle_inversion(&value, &inverted)
                })
            }
        }
        ConstraintExpression::StrContains {
            values,
            case_insensitive,
        } => {
            if values.is_empty() {
                Box::new(|_| false)
            } else {
                let as_vec: Vec<String> = values.to_vec();
                Box::new(move |context: &Context| {
                    let value = getter(context)
                        .map(|v| as_vec.iter().any(|x| v.contains(x)))
                        .unwrap_or(false);
                    _handle_inversion(&value, &inverted)
                })
            }
        }
        ConstraintExpression::NumEq { value } => match value.parse::<f64>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = getter(context)
                    .map(|v| {
                        v.parse::<f64>()
                            .map(|x| (x - parsed_value).abs() < f64::EPSILON)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::NumGt { value } => match value.parse::<f64>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value < constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::NumGte { value } => match value.parse::<f64>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value <= constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::NumLt { value } => match value.parse::<f64>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value > constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::NumLte { value } => match value.parse::<f64>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value >= constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::DateAfter { value } => match DateTime::parse_from_rfc3339(value) {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value < constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::DateBefore { value } => match DateTime::parse_from_rfc3339(value) {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value > constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::SemverEq { value } => match value.parse::<Version>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                let value = _evaluate_ordinal_constraint(
                    getter(context),
                    &parsed_value,
                    |context_value, constraint_value| context_value == constraint_value,
                );
                _handle_inversion(&value, &inverted)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::SemverGt { value } => match value.parse::<Version>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                getter(context)
                    .map(|v| Version::parse(v).map(|x| x > parsed_value).unwrap_or(false))
                    .unwrap_or(false)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
        ConstraintExpression::SemverLt { value } => match value.parse::<Version>() {
            Ok(parsed_value) => Box::new(move |context: &Context| {
                getter(context)
                    .map(|v| Version::parse(v).map(|x| x < parsed_value).unwrap_or(false))
                    .unwrap_or(false)
            }),
            Err(_) => Box::new(move |_| _handle_inversion(&false, &inverted)),
        },
    }
}

fn _ip_to_vec(ips: &[String]) -> Vec<IpNet> {
    let mut result = Vec::new();
    for ip_str in ips {
        let ip_parsed = _parse_ip(ip_str.trim());
        if let Ok(ip) = ip_parsed {
            result.push(ip);
        } else {
            warn!("Could not parse IP address {:?}", ip_str);
        }
    }
    result
}

fn _handle_inversion(value: &bool, inverted: &Option<bool>) -> bool {
    if inverted.unwrap_or(false) {
        !value
    } else {
        *value
    }
}

fn _evaluate_ordinal_constraint<F, T>(
    context_value: Option<&String>,
    constraint_value: &T,
    comparator: F,
) -> bool
where
    T: PartialOrd + FromStr,
    F: Fn(&T, &T) -> bool,
{
    context_value
        .map(|v| {
            v.parse::<T>()
                .map(|x| comparator(constraint_value, &x))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_host<F>(
    expression: ConstraintExpression,
    inverted: Option<bool>,
    getter: F,
) -> Evaluate
where
    F: Fn(&Context) -> Option<&crate::context::IPAddress> + Clone + Sync + Send + 'static,
{
    match &expression {
        ConstraintExpression::In { values } => {
            let ips = _ip_to_vec(values);
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
                let ips = _ip_to_vec(values);
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
                        .unwrap_or(false)
                })
            }
        }
        // New operators aren't defined for an ip
        _ => Box::new(move |_: &Context| false),
    }
}

fn _compile_constraints(constraints: Vec<Constraint>) -> Vec<Evaluate> {
    constraints
        .into_iter()
        .map(|constraint| {
            let (context_name, expression, inverted) = (
                constraint.context_name,
                constraint.expression,
                constraint.inverted,
            );
            match context_name.as_str() {
                "appName" => _compile_constraint_string(expression, inverted, |context| {
                    Some(&context.app_name)
                }),
                "currentTime" => _compile_constraint_string(expression, inverted, |context| {
                    Some(&context.current_time)
                }),
                "environment" => _compile_constraint_string(expression, inverted, |context| {
                    Some(&context.environment)
                }),
                "remoteAddress" => _compile_constraint_host(expression, inverted, |context| {
                    context.remote_address.as_ref()
                }),
                "sessionId" => _compile_constraint_string(expression, inverted, |context| {
                    context.session_id.as_ref()
                }),
                "userId" => _compile_constraint_string(expression, inverted, |context| {
                    context.user_id.as_ref()
                }),
                _ => _compile_constraint_string(expression, inverted, move |context| {
                    context.properties.get(&context_name)
                }),
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
                trace!("constrain: compiling constraints list {:?}", constraints);
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
    use std::collections::hash_map::HashMap;
    use std::default::Default;

    use maplit::hashmap;

    use crate::api::{Constraint, ConstraintExpression};
    use crate::context::{Context, IPAddress};

    fn parse_ip(addr: &str) -> Option<IPAddress> {
        Some(IPAddress(addr.parse().unwrap()))
    }

    #[test]
    fn test_constrain() {
        // Without constraints, things should just pass through
        let context = Context::default();
        assert!(super::constrain(None, &super::default, None)(&context));

        // An empty constraint list acts like a missing one
        let context = Context::default();
        assert!(super::constrain(Some(vec![]), &super::default, None)(
            &context
        ));

        // An empty constraint gets disabled
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "".into(),
                inverted: Some(false),
                expression: ConstraintExpression::In { values: vec![] },
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
                inverted: Some(false),
                expression: ConstraintExpression::In {
                    values: vec!["development".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::NotIn {
                    values: vec!["development".into()]
                },
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
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::In {
                    values: vec!["development".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::In {
                    values: vec!["staging".into(), "development".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::NotIn {
                    values: vec!["staging".into(), "development".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::In {
                    values: vec!["fred".into()]
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::In {
                    values: vec!["qwerty".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::In {
                    values: vec!["10.0.0.0/8".into()],
                },
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
                inverted: Some(false),
                expression: ConstraintExpression::NotIn {
                    values: vec!["10.0.0.0/8".into()],
                },
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
                    inverted: Some(false),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()],
                    },
                },
                Constraint {
                    context_name: "environment".into(),
                    inverted: Some(false),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()],
                    },
                },
            ]),
            &super::default,
            None
        )(&context));
        assert!(!super::constrain(
            Some(vec![
                Constraint {
                    context_name: "environment".into(),
                    inverted: Some(false),
                    expression: ConstraintExpression::In {
                        values: vec!["development".into()],
                    },
                },
                Constraint {
                    inverted: Some(false),
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In { values: vec![] },
                }
            ]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_str_constrain() {
        let context = Context {
            environment: "development".into(),
            ..Default::default()
        };
        // starts with matches string start
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["dev".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // starts with does not match string not present
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrStartsWith {
                    values: vec!["production".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // ends with matches tail of string
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["ent".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // ends with does not match string not present
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrEndsWith {
                    values: vec!["prod".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // contains matches whole string
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrContains {
                    values: vec!["development".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // contains matches partial string
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrContains {
                    values: vec!["elop".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));

        // contains does not match string not present
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::StrContains {
                    values: vec!["production".into()],
                    case_insensitive: Some(false),
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_num_eq_constrain() {
        // Check that this works against floating point contexts
        let context = Context {
            environment: "7.0".into(),
            ..Default::default()
        };

        // eq matches floating point
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq {
                    value: "7.0".into(),
                },
            },]),
            &super::default,
            None
        )(&context));

        // eq does not match against different value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq {
                    value: "3.141".into(),
                },
            },]),
            &super::default,
            None
        )(&context));

        // eq matches against integer
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq { value: "7".into() },
            },]),
            &super::default,
            None
        )(&context));

        // eq returns false for unparsable input
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq {
                    value: "NotANumber".into(),
                },
            },]),
            &super::default,
            None
        )(&context));

        // Check that this works against integer contexts
        let context = Context {
            environment: "7".into(),
            ..Default::default()
        };

        // eq matches against floating point
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq {
                    value: "7.0".into(),
                },
            },]),
            &super::default,
            None
        )(&context));

        // eq matches against integer
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumEq { value: "7".into() },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_num_gt_constrain() {
        let context = Context {
            environment: "14.0".into(),
            ..Default::default()
        };

        // gt does not match against lower value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumGt {
                    value: "17.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gt does not match against exact value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumGt {
                    value: "14.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gt matches against a higher value
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumGt {
                    value: "10.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gt returns false for unparsable value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumGt {
                    value: "NotANumber".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_num_lt_constrain() {
        let context = Context {
            environment: "14.0".into(),
            ..Default::default()
        };

        // lt matches a lower value
        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLt {
                    value: "17.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lt does not match an exact value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLt {
                    value: "14.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lt does not match a higher value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLt {
                    value: "10.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lt returns false for unparsable input
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLt {
                    value: "NotANumber".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_num_gte_constrain() {
        let context = Context {
            environment: "14.0".into(),
            ..Default::default()
        };

        // gte does not match a lower value
        assert!(!super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumGte {
                    value: "17.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gte matches an exact value
        assert!(super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumGte {
                    value: "14.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gte matches a higher value
        assert!(super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumGte {
                    value: "10.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // gte returns false for unparsable input
        assert!(!super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumGte {
                    value: "NotANumber".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_num_lte_constrain() {
        let context = Context {
            environment: "14.0".into(),
            ..Default::default()
        };

        // lte matches a lower value
        assert!(super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumLte {
                    value: "17.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lte matches an exact value
        assert!(super::constrain(
            Some(vec![Constraint {
                inverted: Some(false),
                context_name: "environment".into(),
                expression: ConstraintExpression::NumLte {
                    value: "14.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lte does not match a higher value
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLte {
                    value: "10.0".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        // lte returns false for unparsable input
        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::NumLte {
                    value: "NotANumber".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_semver_eq() {
        let mut props = HashMap::new();
        props.insert("version".into(), "1.2.2".into());
        let context = Context {
            properties: props,
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverEq {
                    value: "1.2.2".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverEq {
                    value: "2.7.1".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "version".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverEq {
                    value: "NotASemver".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_semver_lt() {
        let context = Context {
            environment: "3.1.4-beta.2".into(),
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "6.2.8".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "2.7.1".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "3.1.4-beta.2".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "3.1.4-gamma.3".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "3.1.4-alpha.3".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverLt {
                    value: "NotASemver".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_semver_gt() {
        let context = Context {
            environment: "3.1.4-beta.2".into(),
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "2.7.1".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "6.2.8".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "3.1.4-beta.2".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "3.1.4-gamma.3".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "3.1.4-alpha.3".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "environment".into(),
                inverted: Some(false),
                expression: ConstraintExpression::SemverGt {
                    value: "NotASemver".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_date_before() {
        let context = Context {
            current_time: "2022-01-29T13:00:00.000Z".into(),
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                inverted: Some(false),
                expression: ConstraintExpression::DateBefore {
                    value: "2022-01-30T13:00:00.000Z".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                inverted: Some(false),
                expression: ConstraintExpression::DateBefore {
                    value: "2022-01-28T13:00:00.000Z".into()
                },
            },]),
            &super::default,
            None
        )(&context));
    }

    #[test]
    fn test_date_after() {
        let context = Context {
            current_time: "2022-01-30T13:00:00.000Z".into(),
            ..Default::default()
        };

        assert!(super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                inverted: Some(false),
                expression: ConstraintExpression::DateAfter {
                    value: "2022-01-29T13:00:00.000Z".into()
                },
            },]),
            &super::default,
            None
        )(&context));

        assert!(!super::constrain(
            Some(vec![Constraint {
                context_name: "currentTime".into(),
                inverted: Some(false),
                expression: ConstraintExpression::DateAfter {
                    value: "2022-01-31T13:00:00.000Z".into()
                },
            },]),
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
}
