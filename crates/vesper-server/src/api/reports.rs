//! Reports: CCLI song usage ([`crate::ccli`]).

use super::*;
use axum::extract::Query;
use protocol::CcliReport;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/reports/ccli", get(ccli))
        .route("/reports/ccli.csv", get(ccli_csv))
}

#[derive(serde::Deserialize)]
struct Range {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

/// The range asked for (both days included; default the last 6 months to today). Days are
/// in the years 1900 to 9999 (chrono parses signed years far outside, where date arithmetic
/// panics).
fn range(q: &Range) -> Result<(String, String), Error> {
    use chrono::Datelike;
    let day = |v: &Option<String>, field: &str| match v {
        None => Ok(None),
        Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .filter(|d| (1900..=9999).contains(&d.year()))
            .map(Some)
            .ok_or_else(|| Error(ApiError::invalid(field, "a day, YYYY-MM-DD"))),
    };
    let to = day(&q.to, "to")?.unwrap_or_else(|| chrono::Local::now().date_naive());
    let from = match day(&q.from, "from")? {
        Some(d) => d,
        None => to
            .checked_sub_days(chrono::Days::new(183))
            .unwrap_or(chrono::NaiveDate::MIN),
    };
    if from > to {
        return Err(Error(ApiError::invalid("from", "on or before `to`")));
    }
    Ok((from.to_string(), to.to_string()))
}

async fn report(app: &AppState, q: &Range) -> Result<CcliReport, Error> {
    let (from, to) = range(q)?;
    let sh = app.shared.clone();
    tokio::task::spawn_blocking(move || sh.ccli.report(&from, &to))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))
}

async fn ccli(State(app): State<AppState>, Query(q): Query<Range>) -> ApiResult<CcliReport> {
    ok(report(&app, &q).await?)
}

async fn ccli_csv(State(app): State<AppState>, Query(q): Query<Range>) -> Result<Response, Error> {
    let r = report(&app, &q).await?;
    let name = format!("ccli-usage-{}-to-{}.csv", r.from, r.to);
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        crate::ccli::csv(&r),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(from: Option<&str>, to: Option<&str>) -> Range {
        Range {
            from: from.map(str::to_string),
            to: to.map(str::to_string),
        }
    }

    /// Days far outside the calendar chrono can count back from are refused, not a panic.
    #[test]
    fn ranges_stay_in_sane_years() {
        assert_eq!(
            range(&q(None, Some("2026-08-31"))).ok(),
            Some(("2026-03-01".into(), "2026-08-31".into()))
        );
        assert!(range(&q(None, Some("-262143-01-01"))).is_err());
        assert!(range(&q(Some("2026-01-01"), Some("-262143-01-01"))).is_err());
        assert!(range(&q(Some("0001-01-01"), Some("2026-01-01"))).is_err());
        assert!(range(&q(Some("2026-01-01"), Some("+10000-01-01"))).is_err());
        assert!(range(&q(Some("2026-02-01"), Some("2026-01-01"))).is_err());
        assert!(range(&q(Some("1999-01-01"), Some("2026-01-01"))).is_ok());
    }
}
