use std::{collections::BTreeMap, path::Path};

use crate::errors::StartupError;

/// Operator-owned, case-sensitive category mapping. Tags never select a colour.
#[derive(Debug, Clone)]
pub struct CategoryPalette(BTreeMap<String, String>, BTreeMap<String, String>);

pub const ALLOWED_COLOR_IDS: [&str; 11] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];

pub fn valid_color_id(color: &str) -> bool {
    ALLOWED_COLOR_IDS.contains(&color)
}

impl CategoryPalette {
    pub fn load(path: Option<&Path>) -> Result<Self, StartupError> {
        let mut palette: BTreeMap<String, String> = [
            ("personal", "3"),
            ("relationship", "5"),
            ("business", "6"),
            ("committed", "11"),
            // `location` held colour 8 until P1B-54. It is retired: geographic state belongs in
            // UniverseState, not in a category. An operator's own `calendar.color.location`
            // Setting is still honoured, because a Setting is the operator's record.
            ("sleep", "8"),
            ("entertainment", "1"),
            ("grocery", "2"),
            ("commute", "7"),
            ("undefined", "4"),
            ("education_house", "10"),
            ("work", "9"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let mut origins: BTreeMap<String, String> = palette
            .keys()
            .map(|key| (key.clone(), "default".into()))
            .collect();
        if let Some(path) = path {
            let error = |entry: &str, reason: String| {
                StartupError(format!(
                    "invalid category palette `{}`, entry `{entry}`: {reason}",
                    path.display()
                ))
            };
            let bytes = std::fs::read(path).map_err(|e| error("<file>", e.to_string()))?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| error("<JSON>", e.to_string()))?;
            let entries = value
                .as_object()
                .ok_or_else(|| error("<root>", "expected a JSON object".into()))?;
            for (key, value) in entries {
                let color = value
                    .as_str()
                    .filter(|color| valid_color_id(color))
                    .ok_or_else(|| {
                        error(
                            key,
                            format!("expected a string colorId from \"1\" to \"11\", got {value}"),
                        )
                    })?;
                palette.insert(key.clone(), color.to_owned());
                origins.insert(key.clone(), "file".into());
            }
        }
        Ok(Self(palette, origins))
    }

    /// A validated startup snapshot: file changes still require restart, Setting changes do not.
    pub async fn seed(&self, pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS category_palette_seed (category TEXT PRIMARY KEY, color_id TEXT NOT NULL, origin TEXT NOT NULL)").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM category_palette_seed")
            .execute(&mut *tx)
            .await?;
        for (category, color) in &self.0 {
            sqlx::query(
                "INSERT INTO category_palette_seed (category,color_id,origin) VALUES (?,?,?)",
            )
            .bind(category)
            .bind(color)
            .bind(&self.1[category])
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    /// Compose the default/file startup seed and current admitted Setting overrides.
    pub async fn from_layers(
        pool: &sqlx::SqlitePool,
        settings: &[ubu_store::models::object_record::ObjectRecord],
    ) -> crate::errors::Result<Self> {
        let mut palette = Self(BTreeMap::new(), BTreeMap::new());
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT category,color_id,origin FROM category_palette_seed ORDER BY category",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| crate::errors::AppError::Internal(e.to_string()))?;
        for (category, color, origin) in rows {
            palette.0.insert(category.clone(), color);
            palette.1.insert(category, origin);
        }
        for row in settings.iter().filter(|row| row.status == "active") {
            let value: serde_json::Value = serde_json::from_str(&row.payload_json)
                .map_err(|e| crate::errors::AppError::Internal(e.to_string()))?;
            if let Some(category) = value["name"]
                .as_str()
                .and_then(|name| name.strip_prefix("calendar.color."))
                .filter(|category| !category.is_empty())
            {
                if let Some(color) = value["value"]
                    .as_str()
                    .filter(|color| valid_color_id(color))
                {
                    palette.0.insert(category.to_owned(), color.to_owned());
                    palette.1.insert(category.to_owned(), "setting".into());
                }
            }
        }
        Ok(palette)
    }

    pub async fn from_pool(pool: &sqlx::SqlitePool) -> crate::errors::Result<Self> {
        let settings = crate::services::setting_authoring::settings(pool).await?;
        Self::from_layers(pool, &settings).await
    }

    pub fn entries(&self) -> Vec<crate::api::setting::PaletteEntry> {
        self.0
            .iter()
            .map(|(category, color)| crate::api::setting::PaletteEntry {
                category: category.clone(),
                color_id: color.clone(),
                origin: self.1[category].clone(),
            })
            .collect()
    }

    pub fn inverse_entries(&self) -> Vec<crate::api::setting::InversePaletteEntry> {
        ALLOWED_COLOR_IDS
            .into_iter()
            .map(|color| {
                let categories: Vec<_> = self
                    .0
                    .iter()
                    .filter(|(_, value)| value.as_str() == color)
                    .map(|(key, _)| key.clone())
                    .collect();
                let status = match categories.len() {
                    0 => "unmapped",
                    1 => "mapped",
                    _ => "collision",
                };
                crate::api::setting::InversePaletteEntry {
                    color_id: color.into(),
                    categories,
                    status: status.into(),
                }
            })
            .collect()
    }

    /// None means multiple categories share this colour; never choose arbitrarily.
    pub fn inverse(&self) -> BTreeMap<String, Option<String>> {
        let mut inverse = BTreeMap::new();
        for (category, color) in &self.0 {
            inverse
                .entry(color.clone())
                .and_modify(|value| *value = None)
                .or_insert_with(|| Some(category.clone()));
        }
        inverse
    }

    pub fn color(&self, category: Option<&str>) -> Option<&str> {
        category
            .and_then(|category| self.0.get(category))
            .map(String::as_str)
    }
}
