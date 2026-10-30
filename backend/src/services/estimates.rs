use crate::error::{AutoForgeError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateItem {
    pub description: String,
    pub quantity: u32,
    pub unit_price: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateInput {
    pub title: String,
    pub customer: String,
    pub supplier: String,
    pub issued_on: String,
    pub valid_until: String,
    pub items: Vec<EstimateItem>,
    pub include_vat: bool,
    pub notes: String,
    #[serde(default)]
    pub requirements: String,
    #[serde(default)]
    pub daily_rate: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EstimateGeneration {
    pub requirements: String,
    pub daily_rate: u64,
    pub title: String,
}

impl EstimateGeneration {
    pub fn validate(&self) -> Result<()> {
        if self.requirements.trim().chars().count() < 10
            || self.requirements.len() > 20000
            || self.title.trim().is_empty()
            || self.title.len() > 600
            || self.daily_rate == 0
            || self.daily_rate > 1_000_000_000
        {
            return Err(AutoForgeError::BadRequest(
                "요구사항(10자 이상), 제목, 1일 단가를 확인해주세요.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerationJob {
    pub input: EstimateGeneration,
    pub agent_id: String,
    pub run_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct GeneratedDraft {
    items: Vec<GeneratedItem>,
    assumptions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GeneratedItem {
    description: String,
    days: u32,
}

pub fn parse_generation(text: &str, input: &EstimateGeneration) -> Result<EstimateInput> {
    let value = crate::services::architecture_qa::extract_json_object(text)
        .ok_or_else(|| AutoForgeError::CursorApi("AI 가견적 응답에 JSON이 없습니다.".into()))?;
    let generated: GeneratedDraft = serde_json::from_value(value)
        .map_err(|e| AutoForgeError::CursorApi(format!("AI 가견적 형식 오류: {e}")))?;
    let today = chrono::Utc::now().date_naive();
    let valid = today
        .checked_add_signed(chrono::Duration::days(30))
        .ok_or_else(|| AutoForgeError::Internal("date overflow".into()))?;
    let draft = EstimateInput {
        title: input.title.clone(), customer: String::new(), supplier: String::new(),
        issued_on: today.to_string(), valid_until: valid.to_string(),
        items: generated.items.into_iter().map(|item| EstimateItem {
            description: item.description, quantity: item.days, unit_price: input.daily_rate,
        }).collect(),
        include_vat: true,
        notes: format!("AI가 요구사항을 바탕으로 산정한 참고용 가견적입니다. 범위 확정 후 금액을 검토해주세요.\n수량 단위: 인일 (1인 1일)\n가정 및 제외사항:\n{}", generated.assumptions.join("\n")),
        requirements: input.requirements.clone(),
        daily_rate: Some(input.daily_rate),
    };
    draft.clone().calculate(Uuid::nil())?;
    Ok(draft)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estimate {
    pub id: Uuid,
    #[serde(flatten)]
    pub input: EstimateInput,
    pub subtotal: u64,
    pub vat: u64,
    pub total: u64,
}

impl EstimateInput {
    pub fn calculate(self, id: Uuid) -> Result<Estimate> {
        let invalid =
            || AutoForgeError::BadRequest("견적 항목, 날짜와 금액을 확인해주세요.".into());
        if self.title.trim().is_empty()
            || self.title.len() > 600
            || self.customer.len() > 600
            || self.supplier.len() > 1000
            || self.notes.len() > 12000
            || self.requirements.len() > 20000
            || self
                .daily_rate
                .is_some_and(|rate| rate == 0 || rate > 1_000_000_000)
            || self.items.is_empty()
            || self.items.len() > 100
        {
            return Err(invalid());
        }
        let issued = chrono::NaiveDate::parse_from_str(&self.issued_on, "%Y-%m-%d")
            .map_err(|_| invalid())?;
        let valid = chrono::NaiveDate::parse_from_str(&self.valid_until, "%Y-%m-%d")
            .map_err(|_| invalid())?;
        if valid < issued {
            return Err(invalid());
        }
        let mut subtotal = 0_u64;
        for item in &self.items {
            if item.description.trim().is_empty()
                || item.description.len() > 1500
                || item.quantity == 0
                || item.quantity > 10000
                || item.unit_price > 1_000_000_000
            {
                return Err(invalid());
            }
            subtotal = subtotal
                .checked_add(
                    u64::from(item.quantity)
                        .checked_mul(item.unit_price)
                        .ok_or_else(invalid)?,
                )
                .ok_or_else(invalid)?;
        }
        // KRW is integral; discard the fractional won of the aggregate 10% VAT.
        let vat = if self.include_vat { subtotal / 10 } else { 0 };
        let total = subtotal.checked_add(vat).ok_or_else(invalid)?;
        if total > 1_000_000_000_000 {
            return Err(invalid());
        }
        Ok(Estimate {
            id,
            input: self,
            subtotal,
            vat,
            total,
        })
    }
}

pub async fn list(base: &Path) -> Result<Vec<Estimate>> {
    let mut dir = match tokio::fs::read_dir(base.join("estimates")).await {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AutoForgeError::Store(e.to_string())),
    };
    let mut estimates = Vec::new();
    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| AutoForgeError::Store(e.to_string()))?
    {
        if entry.path().extension().is_some_and(|ext| ext == "json") {
            let data = tokio::fs::read(entry.path())
                .await
                .map_err(|e| AutoForgeError::Store(e.to_string()))?;
            estimates.push(
                serde_json::from_slice::<Estimate>(&data)
                    .map_err(|e| AutoForgeError::Store(e.to_string()))?,
            );
        }
    }
    estimates.sort_by(|a, b| {
        b.input
            .issued_on
            .cmp(&a.input.issued_on)
            .then(a.id.cmp(&b.id))
    });
    Ok(estimates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::artifacts::save_json;

    fn input() -> EstimateInput {
        EstimateInput {
            title: "개발 견적".into(),
            customer: "수신처".into(),
            supplier: "공급자".into(),
            issued_on: "2026-10-30".into(),
            valid_until: "2026-11-30".into(),
            items: vec![EstimateItem {
                description: "개발".into(),
                quantity: 3,
                unit_price: 10001,
            }],
            include_vat: true,
            notes: String::new(),
            requirements: String::new(),
            daily_rate: None,
        }
    }

    #[test]
    fn calculates_aggregate_vat_in_whole_won() {
        let quote = input().calculate(Uuid::new_v4()).unwrap();
        assert_eq!(
            (quote.subtotal, quote.vat, quote.total),
            (30003, 3000, 33003)
        );
    }

    #[test]
    fn excludes_vat_when_unchecked() {
        let mut input = input();
        input.include_vat = false;
        let quote = input.calculate(Uuid::new_v4()).unwrap();
        assert_eq!((quote.vat, quote.total), (0, 30003));
    }

    #[test]
    fn generated_effort_uses_supplied_rate_and_rejects_zero_days() {
        let input = EstimateGeneration {
            title: "웹 개발".into(),
            requirements: "로그인과 관리 화면 개발".into(),
            daily_rate: 400000,
        };
        let draft = parse_generation(
            r#"{"items":[{"description":"로그인","days":3}],"assumptions":["디자인 별도"]}"#,
            &input,
        )
        .unwrap();
        assert_eq!(draft.items[0].unit_price, 400000);
        assert_eq!(draft.calculate(Uuid::nil()).unwrap().total, 1320000);
        assert!(parse_generation(
            r#"{"items":[{"description":"로그인","days":0}],"assumptions":[]}"#,
            &input
        )
        .is_err());
    }

    #[test]
    fn rejects_invalid_money_and_dates() {
        for price in [1_000_000_001, u64::MAX] {
            let mut input = input();
            input.items[0].unit_price = price;
            assert!(input.calculate(Uuid::new_v4()).is_err());
        }
        let mut input = input();
        input.valid_until = "2026-10-29".into();
        assert!(input.calculate(Uuid::new_v4()).is_err());
    }

    #[tokio::test]
    async fn persists_and_replaces_one_estimate() {
        let dir = std::env::temp_dir().join(format!("autoforge-estimates-{}", Uuid::new_v4()));
        assert!(list(&dir).await.unwrap().is_empty());
        let id = Uuid::new_v4();
        let key = format!("estimates/{id}.json");
        let mut quote = input().calculate(id).unwrap();
        save_json(&dir, &key, &quote).await.unwrap();
        quote.input.title = "변경 견적".into();
        save_json(&dir, &key, &quote).await.unwrap();
        let loaded = list(&dir).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].input.title, "변경 견적");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
