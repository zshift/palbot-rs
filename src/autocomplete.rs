use simsearch::SimSearch;

/// A simple autocomplete engine that uses the `simsearch` crate.
pub struct AutoCompleteEngine {
    engine: SimSearch<String>,
}

impl AutoCompleteEngine {
    /// Create a new `AutoCompleteEngine` with the given data.
    pub fn new(data: &[String]) -> Self {
        let mut engine = SimSearch::new();

        for name in data {
            engine.insert(name.clone(), name);
        }

        Self { engine }
    }

    pub fn autocomplete(&self, query: &str) -> Vec<String> {
        self.engine.search(query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autocomplete() {
        let names = vec![
            "Apple".to_string(),
            "Apex".to_string(),
            "Banana".to_string(),
        ];

        let ac = AutoCompleteEngine::new(&names);

        assert_eq!(ac.autocomplete("appl"), vec!["Apple"]);
        assert_eq!(ac.autocomplete("ap"), vec!["Apex", "Apple"]);
    }
}
