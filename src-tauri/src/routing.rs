use crate::models::ModelId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityMode {
    Fast,
    Balanced,
    Maximum,
}

impl QualityMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.eq_ignore_ascii_case("fast") {
            Ok(Self::Fast)
        } else if value.eq_ignore_ascii_case("balanced") {
            Ok(Self::Balanced)
        } else if value.eq_ignore_ascii_case("maximum") {
            Ok(Self::Maximum)
        } else {
            Err("Quality must be Fast, Balanced, or Maximum".into())
        }
    }

    pub fn general_model(self) -> ModelId {
        if self == Self::Maximum {
            ModelId::General
        } else {
            ModelId::GeneralLite
        }
    }
}

pub fn select_model(requested: &str, quality: QualityMode) -> Result<ModelId, String> {
    if requested.eq_ignore_ascii_case("anime") {
        Ok(ModelId::Anime)
    } else if requested.eq_ignore_ascii_case("general") {
        Ok(quality.general_model())
    } else {
        Err("Detection model must be General or Anime".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_uses_the_large_general_model() {
        assert_eq!(QualityMode::Maximum.general_model(), ModelId::General);
        assert_eq!(QualityMode::Fast.general_model(), ModelId::GeneralLite);
    }

    #[test]
    fn only_explicit_models_are_accepted() {
        assert_eq!(
            select_model("General", QualityMode::Balanced).unwrap(),
            ModelId::GeneralLite
        );
        assert_eq!(
            select_model("Anime", QualityMode::Balanced).unwrap(),
            ModelId::Anime
        );
        assert!(select_model("Auto", QualityMode::Balanced).is_err());
    }
}
