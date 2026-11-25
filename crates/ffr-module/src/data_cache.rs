//! Static game-data backends (`ffr-db` SQLite primary backend + `ffr-data` JSON reference cache).
//!
//! Constructing either backend performs no disk I/O so `ffr-module` can always start even when
//! the database file or export directory is absent or incompatible.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use ffr_data::battle::{BattleData, CommandSkillData, EnemyChapterIndex};
use ffr_data::world::WorldData;
use ffr_data::{DataStore, FlagConditionData, MapUnitPlayableData, ScenarioData};
use ffr_db::{DbError, GameDb};

/// Cached domain-availability flags inspected once after opening a [`GameDb`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DbAvailability {
    pub world: bool,
    pub battle: bool,
    pub scenario: bool,
    pub command_skills: bool,
    pub map_unit_playable: bool,
    pub flag_conditions: bool,
    pub enemy_chapters: bool,
}

/// Lazy SQLite backend backed by [`GameDb`].
///
/// Opens the database on first query and caches the shared read-only handle, domain availability
/// flags, and serialized `ffr://` resource payloads.
#[derive(Debug)]
pub struct DbDataCache {
    db_path: Option<PathBuf>,
    db: RwLock<Option<Arc<GameDb>>>,
    availability: RwLock<Option<DbAvailability>>,
    resource_payloads: RwLock<HashMap<String, Arc<String>>>,
}

impl DbDataCache {
    /// Create a lazy database backend bound to an optional SQLite file path.
    pub fn new(db_path: Option<PathBuf>) -> Self {
        Self {
            db_path,
            db: RwLock::new(None),
            availability: RwLock::new(None),
            resource_payloads: RwLock::new(HashMap::new()),
        }
    }

    /// Open and cache the read-only [`GameDb`] handle, returning a Chinese error with rebuild
    /// instructions when unconfigured, missing, or schema-incompatible.
    pub fn open(&self) -> Result<Arc<GameDb>, String> {
        {
            let guard = self
                .db
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(db) = guard.as_ref() {
                return Ok(Arc::clone(db));
            }
        }
        let Some(path) = self.db_path.as_deref() else {
            return Err(
                "未配置游戏数据库：请使用 --db 指定 SQLite 数据库文件，或设置环境变量 FFR_DEPS（默认读取 $FFR_DEPS/ffr/db/ffr.sqlite）；可先运行 `ffr-db build` 构建数据库，或显式传入 --data-dir 使用导出目录"
                    .to_owned(),
            );
        };
        let opened = GameDb::open(path).map_err(format_db_open_error)?;
        let shared = Arc::new(opened);
        let mut guard = self
            .db
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&shared));
        Ok(Arc::clone(cached))
    }

    /// Open the database and return cached domain-availability flags.
    pub fn open_with_availability(&self) -> Result<(Arc<GameDb>, DbAvailability), String> {
        let db = self.open()?;
        {
            let guard = self
                .availability
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(avail) = *guard {
                return Ok((db, avail));
            }
        }
        let computed = DbAvailability {
            world: db.has_world_data().unwrap_or(false),
            battle: db.has_battle_data().unwrap_or(false),
            scenario: db.has_scenario_data().unwrap_or(false),
            command_skills: db.has_command_skills().unwrap_or(false),
            map_unit_playable: db.has_playable_units().unwrap_or(false),
            flag_conditions: db.has_flag_conditions().unwrap_or(false),
            enemy_chapters: db.enemy_chapters_available().unwrap_or(false),
        };
        let mut guard = self
            .availability
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = *guard.get_or_insert(computed);
        Ok((db, cached))
    }

    /// Return a cached serialized resource payload or compute and store it once.
    pub fn get_or_build_resource<F>(&self, key: &str, builder: F) -> Result<Arc<String>, String>
    where
        F: FnOnce(&Self) -> Result<String, String>,
    {
        {
            let guard = self
                .resource_payloads
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(payload) = guard.get(key) {
                return Ok(Arc::clone(payload));
            }
        }
        let built = Arc::new(builder(self)?);
        let mut guard = self
            .resource_payloads
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = guard
            .entry(key.to_owned())
            .or_insert_with(|| Arc::clone(&built));
        Ok(Arc::clone(entry))
    }
}

fn format_db_open_error(error: DbError) -> String {
    match error {
        DbError::OpenFailed { path, detail } => format!(
            "游戏数据库不存在或无法打开（{}）：{detail}；请先运行 `ffr-db build` 构建数据库（默认输出到 $FFR_DEPS/ffr/db/ffr.sqlite），或用 --db 指定有效文件",
            path.display()
        ),
        DbError::SchemaVersionMismatch { .. } | DbError::MissingSchemaVersion { .. } => {
            error.to_string()
        }
        other => format!("打开游戏数据库失败：{other}；请运行 `ffr-db build` 重新构建"),
    }
}

/// Active static game-data backend for [`crate::FfrModule`].
#[derive(Debug)]
pub enum GameDataSource {
    /// Primary SQLite database backend (`--db` or default `$FFR_DEPS/ffr/db/ffr.sqlite`).
    Db(DbDataCache),
    /// Legacy exported-JSON reference backend (enabled only when `--data-dir` is explicitly given).
    Json(GameDataCache),
}

impl GameDataSource {
    /// Construct a lazy SQLite database backend.
    pub fn db(db_path: Option<PathBuf>) -> Self {
        Self::Db(DbDataCache::new(db_path))
    }

    /// Construct the legacy exported-JSON reference backend.
    pub fn json(data_dir: Option<PathBuf>) -> Self {
        Self::Json(GameDataCache::new(data_dir))
    }

    /// Returns `true` when the underlying backend is available for building resource size hints.
    pub fn is_available(&self) -> bool {
        match self {
            Self::Db(db_cache) => db_cache.open().is_ok(),
            Self::Json(cache) => cache.require_data_dir().is_ok(),
        }
    }
}

/// Cached, lazy loader over an optional FFR CUE4Parse export directory.
#[derive(Debug)]
pub struct GameDataCache {
    data_dir: Option<PathBuf>,
    store: RwLock<Option<Arc<DataStore>>>,
    scenario: RwLock<Option<Arc<ScenarioData>>>,
    battle: RwLock<Option<Arc<BattleData>>>,
    world: RwLock<Option<Arc<WorldData>>>,
    command_skills: RwLock<Option<Arc<CommandSkillData>>>,
    map_unit_playable: RwLock<Option<Arc<MapUnitPlayableData>>>,
    flag_conditions: RwLock<Option<Arc<FlagConditionData>>>,
    enemy_chapters: RwLock<Option<Arc<EnemyChapterIndex>>>,
    resource_payloads: RwLock<HashMap<String, Arc<String>>>,
}

impl GameDataCache {
    /// Create a cache bound to an optional export root. No files are read until queried.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            store: RwLock::new(None),
            scenario: RwLock::new(None),
            battle: RwLock::new(None),
            world: RwLock::new(None),
            command_skills: RwLock::new(None),
            map_unit_playable: RwLock::new(None),
            flag_conditions: RwLock::new(None),
            enemy_chapters: RwLock::new(None),
            resource_payloads: RwLock::new(HashMap::new()),
        }
    }

    /// Return the configured export directory if it exists on disk, or a Chinese explanation.
    pub fn require_data_dir(&self) -> Result<&Path, String> {
        let Some(dir) = self.data_dir.as_deref() else {
            return Err(
                "未配置游戏数据目录：请使用 --data-dir 指定导出目录，或设置环境变量 FFR_DEPS（默认读取 $FFR_DEPS/ffr/export）"
                    .to_owned(),
            );
        };
        if !dir.is_dir() {
            return Err(format!(
                "游戏数据目录不存在或不是目录：{}（请确认已部署导出数据，并用 --data-dir 或 FFR_DEPS 指定正确路径）",
                dir.display()
            ));
        }
        Ok(dir)
    }

    /// Obtain or initialize the shared [`DataStore`].
    pub fn store(&self) -> Result<Arc<DataStore>, String> {
        let dir = self.require_data_dir()?;
        {
            let guard = self
                .store
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(store) = guard.as_ref() {
                return Ok(Arc::clone(store));
            }
        }
        let store = Arc::new(DataStore::new(dir));
        let mut guard = self
            .store
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&store));
        Ok(Arc::clone(cached))
    }

    /// Load and cache [`ScenarioData`] on demand.
    pub fn scenario(&self) -> Result<Arc<ScenarioData>, String> {
        {
            let guard = self
                .scenario
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            store
                .scenario()
                .map_err(|error| format!("加载章节与主线剧情数据失败：{error}"))?,
        );
        let mut guard = self
            .scenario
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache [`BattleData`] on demand.
    pub fn battle(&self) -> Result<Arc<BattleData>, String> {
        {
            let guard = self
                .battle
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            store
                .battle()
                .map_err(|error| format!("加载战斗数据失败：{error}"))?,
        );
        let mut guard = self
            .battle
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache [`WorldData`] on demand.
    pub fn world(&self) -> Result<Arc<WorldData>, String> {
        {
            let guard = self
                .world
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            WorldData::from_store(&store).map_err(|error| format!("加载世界数据失败：{error}"))?,
        );
        let mut guard = self
            .world
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache typed [`CommandSkillData`] (`Skill/DT_CommandSkillData`) on demand.
    pub fn command_skills(&self) -> Result<Arc<CommandSkillData>, String> {
        {
            let guard = self
                .command_skills
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            store
                .command_skills()
                .map_err(|error| format!("加载战斗指令技能数据失败：{error}"))?,
        );
        let mut guard = self
            .command_skills
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache typed [`MapUnitPlayableData`] (`Map/DT_MapUnitPlayableData`) on demand.
    pub fn map_unit_playable(&self) -> Result<Arc<MapUnitPlayableData>, String> {
        {
            let guard = self
                .map_unit_playable
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            store
                .map_unit_playable()
                .map_err(|error| format!("加载野外可操作角色数据失败：{error}"))?,
        );
        let mut guard = self
            .map_unit_playable
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache typed [`FlagConditionData`] (`Flag/CDT_FlagCondition`) on demand.
    pub fn flag_conditions(&self) -> Result<Arc<FlagConditionData>, String> {
        {
            let guard = self
                .flag_conditions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let loaded = Arc::new(
            store
                .flag_conditions()
                .map_err(|error| format!("加载事件旗标条件数据失败：{error}"))?,
        );
        let mut guard = self
            .flag_conditions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Load and cache derived [`EnemyChapterIndex`] (Enemy -> Chapter) on demand.
    pub fn enemy_chapters(&self) -> Result<Arc<EnemyChapterIndex>, String> {
        {
            let guard = self
                .enemy_chapters
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(data) = guard.as_ref() {
                return Ok(Arc::clone(data));
            }
        }
        let store = self.store()?;
        let battle = self.battle()?;
        let loaded = Arc::new(
            EnemyChapterIndex::from_store_with_battle(&store, &battle)
                .map_err(|error| format!("构建敌人与章节映射索引失败：{error}"))?,
        );
        let mut guard = self
            .enemy_chapters
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = guard.get_or_insert_with(|| Arc::clone(&loaded));
        Ok(Arc::clone(cached))
    }

    /// Return a cached serialized resource payload or compute and store it once.
    pub fn get_or_build_resource<F>(&self, key: &str, builder: F) -> Result<Arc<String>, String>
    where
        F: FnOnce(&Self) -> Result<String, String>,
    {
        {
            let guard = self
                .resource_payloads
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(payload) = guard.get(key) {
                return Ok(Arc::clone(payload));
            }
        }
        let built = Arc::new(builder(self)?);
        let mut guard = self
            .resource_payloads
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = guard
            .entry(key.to_owned())
            .or_insert_with(|| Arc::clone(&built));
        Ok(Arc::clone(entry))
    }
}
