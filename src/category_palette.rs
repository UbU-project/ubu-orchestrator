use std::{collections::BTreeMap, path::Path};

use crate::errors::StartupError;

/// Operator-owned, case-sensitive category mapping. Tags never select a colour.
#[derive(Debug, Clone)]
pub struct CategoryPalette(BTreeMap<String, String>);

impl CategoryPalette {
    pub fn load(path: Option<&Path>) -> Result<Self, StartupError> {
        let mut palette: BTreeMap<String, String> = [
            ("personal", "3"),
            ("relationship", "5"),
            ("business", "6"),
            ("committed", "11"),
            ("location", "8"),
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
                    .filter(|color| {
                        matches!(
                            *color,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11"
                        )
                    })
                    .ok_or_else(|| {
                        error(
                            key,
                            format!("expected a string colorId from \"1\" to \"11\", got {value}"),
                        )
                    })?;
                palette.insert(key.clone(), color.to_owned());
            }
        }
        Ok(Self(palette))
    }

    pub fn color(&self, category: Option<&str>) -> Option<&str> {
        category
            .and_then(|category| self.0.get(category))
            .map(String::as_str)
    }
}
