//! 应用管理正式／测试平台入口；发行构建不接受外部地址覆盖。
use super::transport;
use crate::output::{Failure, Result, invalid};

#[cfg(test)]
mod tests;

/// 用户未指定运行环境时使用正式环境。
/// 正式环境开放平台入口，不包含授权路径或查询参数。
const PRODUCTION_ORIGIN: &str = "https://open-platform.jmj1995.com";
/// 测试环境开放平台入口，不包含授权路径或查询参数。
const STAGING_ORIGIN: &str = "https://open-platform-stg.datousoft.com";

enum Environment {
    /// 正式业务环境。
    Production,
    /// 集成测试环境。
    Staging,
    #[cfg(any(test, feature = "local-dev"))]
    /// 本机隔离联调环境；不会进入原生发行构建。
    Local {
        /// 隔离服务约定的环境标识，不得替代正式或测试环境。
        name: String,
    },
}

pub(super) struct PlatformEndpoint {
    /// 应用已核对的运行环境。
    environment: Environment,
    /// 对应环境的平台入口；正式／测试环境始终取应用常量。
    origin: String,
}

impl PlatformEndpoint {
    pub(super) fn environment(&self) -> &str {
        match &self.environment {
            Environment::Production => "prod",
            Environment::Staging => "stg",
            #[cfg(any(test, feature = "local-dev"))]
            Environment::Local { name } => name,
        }
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }
}

fn origin_mismatch() -> Failure {
    let mut failure = Failure::new(
        "PLATFORM_ORIGIN_MISMATCH",
        2,
        "平台入口由应用按环境固定，地址与所选环境不匹配。",
    );
    failure.hint =
        "请移除 --origin；正式环境使用 --environment prod，测试环境使用 --environment stg。";
    failure
}

/// 记录地址仅作一致性校验，不参与正式／测试入口的选择。
pub(super) fn resolve(environment: &str, record_origin: Option<&str>) -> Result<PlatformEndpoint> {
    if environment.is_empty()
        || environment.len() > 100
        || !environment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte))
    {
        return Err(invalid());
    }
    if let Some(origin) = record_origin {
        transport::origin(origin)?;
    }
    let (selected, origin) = match environment {
        "prod" => (Environment::Production, PRODUCTION_ORIGIN),
        "stg" => (Environment::Staging, STAGING_ORIGIN),
        _ => {
            #[cfg(any(test, feature = "local-dev"))]
            if let Some(origin) = record_origin
                && url::Url::parse(origin)
                    .is_ok_and(|url| url.scheme() == "http" && url.host_str() == Some("127.0.0.1"))
            {
                return Ok(PlatformEndpoint {
                    environment: Environment::Local {
                        name: environment.to_owned(),
                    },
                    origin: origin.to_owned(),
                });
            }
            let mut failure = Failure::new(
                "PLATFORM_ENVIRONMENT_UNSUPPORTED",
                2,
                "应用只支持正式环境 prod 和测试环境 stg。",
            );
            failure.hint = "使用 --environment prod 或 --environment stg；未指定时默认正式环境。";
            return Err(failure);
        }
    };
    if record_origin.is_some_and(|value| value != origin) {
        return Err(origin_mismatch());
    }
    Ok(PlatformEndpoint {
        environment: selected,
        origin: origin.to_owned(),
    })
}

/// 使用旧账号记录前核对入口，避免凭证被发往记录中保存的其他站点。
pub(super) fn validate_profile(origin: &str, environment: &str) -> Result<()> {
    resolve(environment, Some(origin))
        .map(|_| ())
        .map_err(|failure| {
            if matches!(
                failure.code,
                "PLATFORM_ORIGIN_MISMATCH" | "PLATFORM_ENVIRONMENT_UNSUPPORTED"
            ) {
                let mut failure = Failure::new(
                    "PLATFORM_PROFILE_ORIGIN_MISMATCH",
                    2,
                    "已有平台记录的入口与应用环境配置不匹配，原记录已保留。",
                );
                failure.hint = "请使用未占用的本地记录名，登录对应的正式或测试环境。";
                failure
            } else {
                failure
            }
        })
}
