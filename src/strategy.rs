// Copyright 2020 Cognite AS
//! https://unleash.github.io/docs/activation_strategy
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::hash::BuildHasher;
use std::io::Cursor;

use log::{trace, warn};
use murmur3::murmur3_32;
use rand::Rng;

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

/// https://unleash.github.io/docs/activation_strategy#default
pub fn default<S: BuildHasher>(_: Option<HashMap<String, String, S>>) -> Evaluate {
    Box::new(|_: &Context| -> bool { true })
}

/// https://unleash.github.io/docs/activation_strategy#userwithid
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

/// https://unleash.github.io/docs/activation_strategy#flexiblerollout
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

/// https://unleash.github.io/docs/activation_strategy#gradualrolloutuserid
/// percentage: 0-100
/// groupId: hash key
pub fn user_id<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _user_id(parameters, "percentage")
}

/// https://unleash.github.io/docs/activation_strategy#gradualrolloutsessionid
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

/// https://unleash.github.io/docs/activation_strategy#gradualrolloutrandom
/// percentage: percentage 0-100
pub fn random<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _random(parameters, "percentage")
}

/// https://unleash.github.io/docs/activation_strategy#remoteaddress
/// IPs: 1.2.3.4,AB::CD::::EF,1.2/8
pub fn remote_address<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    // TODO: this could be optimised given the inherent radix structure, but its
    // not exactly hot-path.
    let mut ips: Vec<ipaddress::IPAddress> = Vec::new();
    if let Some(parameters) = parameters {
        if let Some(ips_str) = parameters.get("IPs") {
            for ip_str in ips_str.split(',') {
                let ip_parsed = ipaddress::IPAddress::parse(ip_str.trim());
                if let Ok(ip) = ip_parsed {
                    ips.push(ip)
                }
            }
        }
    }

    Box::new(move |context: &Context| -> bool {
        if let Some(remote_address) = &context.remote_address {
            for ip in &ips {
                if ip.includes(&remote_address.0) {
                    return true;
                }
            }
        }
        false
    })
}

/// https://unleash.github.io/docs/activation_strategy#applicationhostname
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
fn _compile_constraint_string<F>(expression: ConstraintExpression, getter: F) -> Evaluate
where
    F: Fn(&Context) -> Option<&String> + Clone + Sync + Send + 'static,
{
    match &expression {
        ConstraintExpression::In(values) => {
            let as_set: HashSet<String> = values.iter().cloned().collect();
            Box::new(move |context: &Context| {
                getter(context).map(|v| as_set.contains(v)).unwrap_or(false)
            })
        }
        ConstraintExpression::NotIn(values) => {
            if values.is_empty() {
                Box::new(|_| true)
            } else {
                let as_set: HashSet<String> = values.iter().cloned().collect();
                Box::new(move |context: &Context| {
                    getter(context)
                        .map(|v| !as_set.contains(v))
                        .unwrap_or(false)
                })
            }
        }
    }
}

fn _ip_to_vec(ips: &[String]) -> Vec<ipaddress::IPAddress> {
    let mut result = Vec::new();
    for ip_str in ips {
        let ip_parsed = ipaddress::IPAddress::parse(ip_str.trim());
        if let Ok(ip) = ip_parsed {
            result.push(ip);
        } else {
            warn!("Could not parse IP address {:?}", ip_str);
        }
    }
    result
}

/// returns true if the strategy should be delegated to, false to disable
fn _compile_constraint_host<F>(expression: ConstraintExpression, getter: F) -> Evaluate
where
    F: Fn(&Context) -> Option<&crate::context::IPAddress> + Clone + Sync + Send + 'static,
{
    match &expression {
        ConstraintExpression::In(values) => {
            let ips = _ip_to_vec(values);
            Box::new(move |context: &Context| {
                getter(context)
                    .map(|remote_address| {
                        for ip in &ips {
                            if ip.includes(&remote_address.0) {
                                return true;
                            }
                        }
                        false
                    })
                    .unwrap_or(false)
            })
        }
        ConstraintExpression::NotIn(values) => {
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
                                if ip.includes(&remote_address.0) {
                                    return false;
                                }
                            }
                            true
                        })
                        .unwrap_or(false)
                })
            }
        }
    }
}

fn _compile_constraints(constraints: Vec<Constraint>) -> Vec<Evaluate> {
    constraints
        .into_iter()
        .map(|constraint| {
            let (context_name, expression) = (constraint.context_name, constraint.expression);
            match context_name.as_str() {
                "appName" => {
                    _compile_constraint_string(expression, |context| Some(&context.app_name))
                }
                "environment" => {
                    _compile_constraint_string(expression, |context| Some(&context.environment))
                }
                "remoteAddress" => {
                    _compile_constraint_host(expression, |context| context.remote_address.as_ref())
                }
                "sessionId" => {
                    _compile_constraint_string(expression, |context| context.session_id.as_ref())
                }
                "userId" => {
                    _compile_constraint_string(expression, |context| context.user_id.as_ref())
                }
                _ => _compile_constraint_string(expression, move |context| {
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

#[cfg(test)]
mod tests {
    use std::collections::hash_map::HashMap;
    use std::default::Default;

    use maplit::hashmap;

    use crate::api::{Constraint, ConstraintExpression};
    use crate::context::{Context, IPAddress};

    fn parse_ip(addr: &str) -> Option<IPAddress> {
        Some(IPAddress(ipaddress::IPAddress::parse(addr).unwrap()))
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
                expression: ConstraintExpression::In(vec![]),
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
                expression: ConstraintExpression::In(vec!["development".into()]),
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
                expression: ConstraintExpression::NotIn(vec!["development".into()]),
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
                expression: ConstraintExpression::In(vec!["development".into()]),
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
                expression: ConstraintExpression::In(vec!["staging".into(), "development".into()]),
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
                expression: ConstraintExpression::NotIn(vec![
                    "staging".into(),
                    "development".into()
                ]),
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
                expression: ConstraintExpression::In(vec!["fred".into()]),
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
                expression: ConstraintExpression::In(vec!["qwerty".into()]),
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
                expression: ConstraintExpression::In(vec!["10.0.0.0/8".into()]),
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
                expression: ConstraintExpression::NotIn(vec!["10.0.0.0/8".into()]),
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
                    expression: ConstraintExpression::In(vec!["development".into()]),
                },
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In(vec!["development".into()]),
                },
            ]),
            &super::default,
            None
        )(&context));
        assert!(!super::constrain(
            Some(vec![
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In(vec!["development".into()]),
                },
                Constraint {
                    context_name: "environment".into(),
                    expression: ConstraintExpression::In(vec![]),
                }
            ]),
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
            "IPs".into() => "1.2/8,2.3.4.5,2222:FF:0:1234::/64".into()
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
