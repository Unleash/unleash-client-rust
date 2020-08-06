// Copyright 2020 Cognite AS
//! https://unleash.github.io/docs/activation_strategy
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::hash::BuildHasher;
use std::io::Cursor;
use std::io::Read;

use murmur3::murmur3_32;
use rand::Rng;

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
                uids.insert(uid.into());
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
    let mut reader = Cursor::new(&group)
        .chain(Cursor::new(":"))
        .chain(Cursor::new(&variable));
    if let Ok(hash_result) = murmur3_32(&mut reader, 0) {
        let normalised = hash_result % 100;
        rollout > normalised
    } else {
        false
    }
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
/// stickiness: [DEFAULT|USERID|SESSIONID|RANDOM]
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
        "DEFAULT" => {
            // user, session, random in that order.
            let (group, rollout) = group_and_rollout(&parameters, "rollout");
            Box::new(move |context: &Context| -> bool {
                if context.user_id.is_some() {
                    partial_rollout(&group, context.user_id.as_ref(), rollout)
                } else if context.session_id.is_some() {
                    partial_rollout(&group, context.session_id.as_ref(), rollout)
                } else {
                    let picked = rand::thread_rng().gen_range(0, 100);
                    rollout > picked
                }
            })
        }
        "USERID" => _user_id(parameters, "rollout"),
        "SESSIONID" => _session_id(parameters, "rollout"),
        "RANDOM" => _random(parameters, "rollout"),
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
        let picked = rng.gen_range(0, 100);
        pct > picked
    })
}

/// https://unleash.github.io/docs/activation_strategy#gradualrolloutrandom
/// percentage: percentage 0-100
pub fn random<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    _random(parameters, "percentage")
}

/// https://unleash.github.io/docs/activation_strategy#remoteaddress
/// IPS: 1.2.3.4,AB::CD::::EF,1.2/8
pub fn remote_address<S: BuildHasher>(parameters: Option<HashMap<String, String, S>>) -> Evaluate {
    // TODO: this could be optimised given the inherent radix structure, but its
    // not exactly hot-path.
    let mut ips: Vec<ipaddress::IPAddress> = Vec::new();
    if let Some(parameters) = parameters {
        if let Some(ips_str) = parameters.get("IPS") {
            for ip_str in ips_str.split(',') {
                let ip_parsed = ipaddress::IPAddress::parse(ip_str);
                if let Ok(ip) = ip_parsed {
                    ips.push(ip)
                }
            }
        }
    }

    Box::new(move |context: &Context| -> bool {
        if let Some(remote_address) = &context.remote_address {
            for ip in &ips {
                if ip.includes(&remote_address) {
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
                    if this_hostname == hostname {
                        result = true;
                    }
                }
                false
            })
        })
    });

    Box::new(move |_: &Context| -> bool { result })
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::HashMap;
    use std::default::Default;

    use ipaddress::IPAddress;
    use maplit::hashmap;

    use crate::context::Context;

    #[test]
    fn test_user_with_id() {
        let params: HashMap<String, String> = hashmap! {
            "userIds".into() => "fred,barney".into(),
        };
        assert_eq!(
            true,
            super::user_with_id(Some(params.clone()))(&Context {
                user_id: Some("fred".into()),
                ..Default::default()
            })
        );
        assert_eq!(
            true,
            super::user_with_id(Some(params.clone()))(&Context {
                user_id: Some("barney".into()),
                ..Default::default()
            })
        );
        assert_eq!(
            false,
            super::user_with_id(Some(params))(&Context {
                user_id: Some("betty".into()),
                ..Default::default()
            })
        );
    }
    #[test]
    fn test_flexible_rollout() {
        // RANDOM
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "RANDOM".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Default::default();
        assert_eq!(false, super::flexible_rollout(Some(params))(&c));

        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "RANDOM".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Default::default();
        assert_eq!(true, super::flexible_rollout(Some(params))(&c));

        // Could parameterise this by SESSION and USER, but its barely long
        // enough to bother and the explicitness in failures has merit.
        // SESSIONID
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "SESSIONID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "SESSIONID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params))(&c));
        // Check rollout works
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "SESSIONID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            session_id: Some("session3".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params))(&c));
        // Check groupId modifies the hash order
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "SESSIONID".into(),
            "groupId".into() => "group3".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            session_id: Some("session1".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            session_id: Some("session3".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params))(&c));

        // USERID
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "USERID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "0".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "USERID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "100".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params))(&c));
        // Check rollout works
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "USERID".into(),
            "groupId".into() => "group1".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            user_id: Some("user3".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params))(&c));
        // Check groupId modifies the hash order
        let params: HashMap<String, String> = hashmap! {
            "stickiness".into() => "USERID".into(),
            "groupId".into() => "group2".into(),
            "rollout".into() => "50".into(),
        };
        let c: Context = Context {
            user_id: Some("user1".into()),
            ..Default::default()
        };
        assert_eq!(false, super::flexible_rollout(Some(params.clone()))(&c));
        let c: Context = Context {
            user_id: Some("user3".into()),
            ..Default::default()
        };
        assert_eq!(true, super::flexible_rollout(Some(params))(&c));
    }

    #[test]
    fn test_random() {
        let params: HashMap<String, String> = hashmap! {
            "percentage".into() => "0".into()
        };
        let c: Context = Default::default();
        assert_eq!(false, super::random(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "percentage".into() => "100".into()
        };
        let c: Context = Default::default();
        assert_eq!(true, super::random(Some(params))(&c));
    }

    #[test]
    fn test_remote_address() {
        let params: HashMap<String, String> = hashmap! {
            "IPS".into() => "1.2/8,2.3.4.5,2222:FF:0:1234::/64".into()
        };
        let c: Context = Context {
            remote_address: Some(IPAddress::parse("1.2.3.4").unwrap()),
            ..Default::default()
        };
        assert_eq!(true, super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: Some(IPAddress::parse("2.3.4.5").unwrap()),
            ..Default::default()
        };
        assert_eq!(true, super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: Some(IPAddress::parse("2222:FF:0:1234::FDEC").unwrap()),
            ..Default::default()
        };
        assert_eq!(true, super::remote_address(Some(params.clone()))(&c));
        let c: Context = Context {
            remote_address: Some(IPAddress::parse("2.3.4.4").unwrap()),
            ..Default::default()
        };
        assert_eq!(false, super::remote_address(Some(params))(&c));
    }

    #[test]
    fn test_hostname() {
        let c: Context = Default::default();
        let this_hostname = hostname::get().unwrap().into_string().unwrap();
        let params: HashMap<String, String> = hashmap! {
            "hostNames".into() => format!("foo,{},bar", this_hostname)
        };
        assert_eq!(true, super::hostname(Some(params))(&c));
        let params: HashMap<String, String> = hashmap! {
            "hostNames".into() => "foo,bar".into()
        };
        assert_eq!(false, super::hostname(Some(params))(&c));
    }
}
