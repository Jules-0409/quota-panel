//! 配置持久化：把「自动刷新间隔 + 两个颜色阈值」落到用户配置目录。
//!
//! 为什么单独开一个模块：这三个数字以前只活在 `AppConfig::default()` 和
//! `ui/index.html` 里，重启就回默认值（README 的「已知限制」第一条）。现在读写一个 JSON 文件。
//!
//! **这个文件只有这三个数字，没有任何凭据。** 凭据全程只在内存里用完即弃
//! （见 AGENTS.md「只读、不落盘、不伪装」）：能落盘 ≠ 可以把 token 写进来。
//!
//! 读写接口都是收 `&Path` 的纯函数，不碰 `AppHandle`，这样单测可以直接用临时目录
//! 跑一遍真实读写（本机登录态的取数逻辑测不了，这些能测的就必须测）。

use std::fs;
use std::path::{Path, PathBuf};

use crate::models::AppConfig;

/// 文件名。落在 Tauri 的应用配置目录下，
/// macOS 上是 `~/Library/Application Support/<identifier>/config.json`。
const FILE_NAME: &str = "config.json";

/// 刷新间隔下限：0 会把轮询变成打接口的死循环
pub const MIN_REFRESH_MINUTES: u64 = 1;
/// 刷新间隔上限：再长用户会以为程序没在跑
pub const MAX_REFRESH_MINUTES: u64 = 60;

/// 配置文件的完整路径
pub fn config_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// 把配置收敛回合法区间。
///
/// 输入有两个来源：设置界面（点得出越界值）和手改过的配置文件。不收敛的话
/// `refresh_minutes = 0` 会变成死循环，`danger <= warn` 会让界面永远不显示红色。
pub fn sanitize(mut cfg: AppConfig) -> AppConfig {
    cfg.refresh_minutes = cfg
        .refresh_minutes
        .clamp(MIN_REFRESH_MINUTES, MAX_REFRESH_MINUTES);

    // NaN / inf 不可能从 JSON 里读出来，但 set_config 的入参来自前端，得防一手
    if !cfg.warn_percent.is_finite() {
        cfg.warn_percent = AppConfig::default().warn_percent;
    }
    if !cfg.danger_percent.is_finite() {
        cfg.danger_percent = AppConfig::default().danger_percent;
    }

    cfg.warn_percent = cfg.warn_percent.clamp(1.0, 99.0);
    cfg.danger_percent = cfg.danger_percent.clamp(2.0, 100.0);
    if cfg.danger_percent <= cfg.warn_percent {
        // 危险线必须严格高于警告线：相等的话「警告色」这一档永远不会出现
        cfg.danger_percent = (cfg.warn_percent + 1.0).min(100.0);
    }
    cfg
}

/// 读配置。文件不存在 / 读不动 / JSON 坏掉，一律回默认值：
/// 一个坏掉的配置文件不该让小组件起不来，用户删掉它就恢复默认。
pub fn load(path: &Path) -> AppConfig {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(cfg) => sanitize(cfg),
            Err(e) => {
                eprintln!("config: {} 解析失败，改用默认值: {e}", path.display());
                AppConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppConfig::default(),
        Err(e) => {
            eprintln!("config: {} 读取失败，改用默认值: {e}", path.display());
            AppConfig::default()
        }
    }
}

/// 写配置：先写同目录的临时文件再 rename。
///
/// 直接截断写的话，中途失败（磁盘满、被强杀）会留下半截 JSON，
/// 下次启动就读不回用户设置；rename 是原子的，要么旧值要么新值。
pub fn save(path: &Path, cfg: &AppConfig) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    text.push('\n');

    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例一个独立目录，测完自己清掉（并行跑也不会互相踩）
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("quota-panel-config-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trips_and_creates_missing_directories() {
        let dir = tmp_dir("roundtrip");
        let path = config_path(&dir);

        let cfg = AppConfig {
            refresh_minutes: 15,
            warn_percent: 60.0,
            danger_percent: 85.0,
        };
        save(&path, &cfg).unwrap();

        let back = load(&path);
        assert_eq!(back.refresh_minutes, 15);
        assert_eq!(back.warn_percent, 60.0);
        assert_eq!(back.danger_percent, 85.0);

        // 落盘的是 camelCase：和前后端契约同一套字段名，配置文件手写时也照这个来
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"refreshMinutes\""),
            "落盘字段名应为 camelCase: {text}"
        );
        assert!(
            !text.contains("refresh_minutes"),
            "落盘不应出现 snake_case: {text}"
        );
        assert!(text.ends_with('\n'), "文件应以换行结尾，方便 cat/编辑器");

        // 临时文件不能留在磁盘上
        assert!(!path.with_extension("json.tmp").exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let dir = tmp_dir("missing");
        let cfg = load(&config_path(&dir));
        let def = AppConfig::default();
        assert_eq!(cfg.refresh_minutes, def.refresh_minutes);
        assert_eq!(cfg.warn_percent, def.warn_percent);
        assert_eq!(cfg.danger_percent, def.danger_percent);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tmp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = config_path(&dir);
        fs::write(&path, "{ this is not json").unwrap();

        let cfg = load(&path);
        assert_eq!(cfg.refresh_minutes, AppConfig::default().refresh_minutes);
        assert_eq!(cfg.warn_percent, AppConfig::default().warn_percent);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let cfg = sanitize(AppConfig {
            refresh_minutes: 0,
            warn_percent: -5.0,
            danger_percent: 999.0,
        });
        assert_eq!(cfg.refresh_minutes, MIN_REFRESH_MINUTES);
        assert_eq!(cfg.warn_percent, 1.0);
        assert_eq!(cfg.danger_percent, 100.0);

        let cfg = sanitize(AppConfig {
            refresh_minutes: 9_999,
            warn_percent: 100.0,
            danger_percent: 0.0,
        });
        assert_eq!(cfg.refresh_minutes, MAX_REFRESH_MINUTES);
        assert_eq!(cfg.warn_percent, 99.0);
        assert_eq!(cfg.danger_percent, 100.0);
    }

    #[test]
    fn danger_stays_strictly_above_warn() {
        let cfg = sanitize(AppConfig {
            refresh_minutes: 5,
            warn_percent: 80.0,
            danger_percent: 50.0,
        });
        assert_eq!(cfg.warn_percent, 80.0);
        assert_eq!(cfg.danger_percent, 81.0);

        // 相等也不行，否则「警告」这一档永远不出现
        let cfg = sanitize(AppConfig {
            refresh_minutes: 5,
            warn_percent: 90.0,
            danger_percent: 90.0,
        });
        assert_eq!(cfg.danger_percent, 91.0);
    }

    #[test]
    fn non_finite_thresholds_fall_back_to_defaults() {
        let cfg = sanitize(AppConfig {
            refresh_minutes: 5,
            warn_percent: f64::NAN,
            danger_percent: f64::INFINITY,
        });
        assert_eq!(cfg.warn_percent, AppConfig::default().warn_percent);
        assert_eq!(cfg.danger_percent, AppConfig::default().danger_percent);
    }

    #[test]
    fn load_sanitizes_hand_edited_file() {
        let dir = tmp_dir("hand-edited");
        fs::create_dir_all(&dir).unwrap();
        let path = config_path(&dir);
        // 手改出来的越界值：读的时候就要收敛，不能等界面渲染才发现
        fs::write(
            &path,
            r#"{"refreshMinutes":0,"warnPercent":95.0,"dangerPercent":90.0}"#,
        )
        .unwrap();

        let cfg = load(&path);
        assert_eq!(cfg.refresh_minutes, 1);
        assert_eq!(cfg.warn_percent, 95.0);
        assert_eq!(cfg.danger_percent, 96.0);

        fs::remove_dir_all(&dir).unwrap();
    }
}
