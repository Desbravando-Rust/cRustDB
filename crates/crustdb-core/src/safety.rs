use crate::sql::{Classified, DestructiveKind, DmlKind, StatementClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyPolicy {
    pub threshold: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateNeed {
    None,
    Explain,
    TableRows,
}

pub fn estimate_need(c: &Classified) -> EstimateNeed {
    match c.class {
        // No EXPLAIN exists for these; the estimate stays unknown (-> Password).
        StatementClass::Dml(DmlKind::Copy | DmlKind::LoadData) => EstimateNeed::None,
        StatementClass::Dml(_) if c.values_rows.is_some() => EstimateNeed::None,
        StatementClass::Dml(_) => EstimateNeed::Explain,
        StatementClass::Destructive(DestructiveKind::DropTable | DestructiveKind::Truncate) => {
            EstimateNeed::TableRows
        }
        _ => EstimateNeed::None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub index: usize,
    pub sql: String,
    pub classified: Classified,
    pub estimate: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Run,
    Confirm,
    Password,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagReason {
    NoWhere,
    MultiRow(u64),
    OverThreshold(u64),
    UnknownEstimate,
    Destructive(DestructiveKind),
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    pub index: usize,
    pub verdict: Verdict,
    pub reason: FlagReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub verdict: Verdict,
    pub flags: Vec<Flag>,
}

impl SafetyPolicy {
    pub fn evaluate(&self, items: &[Assessment]) -> Evaluation {
        let flags: Vec<Flag> = items.iter().filter_map(|a| self.flag(a)).collect();
        let verdict = flags
            .iter()
            .map(|f| f.verdict)
            .max()
            .unwrap_or(Verdict::Run);
        Evaluation { verdict, flags }
    }

    fn flag(&self, a: &Assessment) -> Option<Flag> {
        let c = &a.classified;
        let e = a.estimate.or(c.values_rows);
        let over = || match e {
            None => Some(FlagReason::UnknownEstimate),
            Some(n) if n > self.threshold => Some(FlagReason::OverThreshold(n)),
            Some(_) => None,
        };
        let (verdict, reason) = match c.class {
            StatementClass::Dml(kind) => {
                if let Some(r) = over() {
                    (Verdict::Password, r)
                } else if !c.has_where && matches!(kind, DmlKind::Update | DmlKind::Delete) {
                    (Verdict::Confirm, FlagReason::NoWhere)
                } else {
                    (
                        Verdict::Confirm,
                        FlagReason::MultiRow(e.filter(|&n| n > 1)?),
                    )
                }
            }
            StatementClass::Destructive(
                k @ (DestructiveKind::DropTable | DestructiveKind::Truncate),
            ) => match over() {
                Some(r) => (Verdict::Password, r),
                None => (Verdict::Confirm, FlagReason::Destructive(k)),
            },
            StatementClass::Destructive(k) => (Verdict::Confirm, FlagReason::Destructive(k)),
            StatementClass::Unknown => (Verdict::Confirm, FlagReason::Unrecognized),
            _ => return None,
        };
        Some(Flag {
            index: a.index,
            verdict,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::model::DriverKind;
    use crate::sql::classify;

    fn eval_batch(items: &[(&str, Option<u64>)], threshold: u64) -> Evaluation {
        let a: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(index, (sql, estimate))| Assessment {
                index,
                sql: sql.to_string(),
                classified: classify(sql, DriverKind::Postgres),
                estimate: *estimate,
            })
            .collect();
        SafetyPolicy { threshold }.evaluate(&a)
    }
    fn eval(sql: &str, estimate: Option<u64>, threshold: u64) -> Evaluation {
        eval_batch(&[(sql, estimate)], threshold)
    }
    fn verdict(sql: &str, estimate: Option<u64>) -> Verdict {
        eval(sql, estimate, 1000).verdict
    }

    #[test]
    fn dml_over_threshold_requires_password() {
        // Act & Assert
        assert_eq!(
            verdict("UPDATE t SET a=1 WHERE b>0", Some(1001)),
            Verdict::Password
        );
    }
    #[test]
    fn unknown_estimate_requires_password() {
        // Act & Assert
        assert_eq!(verdict("DELETE FROM t WHERE a=1", None), Verdict::Password);
    }
    #[test]
    fn multi_row_requires_confirm() {
        // Act & Assert
        assert_eq!(
            verdict("DELETE FROM t WHERE a>1", Some(2)),
            Verdict::Confirm
        );
    }
    #[test]
    fn no_where_requires_confirm_even_if_small() {
        // Act & Assert
        assert_eq!(verdict("DELETE FROM t", Some(1)), Verdict::Confirm);
    }
    #[test]
    fn single_row_with_where_runs() {
        // Act & Assert
        assert_eq!(
            verdict("UPDATE t SET a=1 WHERE id=7", Some(1)),
            Verdict::Run
        );
    }
    #[test]
    fn insert_values_uses_tuple_count() {
        // Act & Assert
        let c = classify("INSERT INTO t VALUES (1)", DriverKind::Mysql);
        assert_eq!(estimate_need(&c), EstimateNeed::None);
        assert_eq!(verdict("INSERT INTO t VALUES (1)", None), Verdict::Run);
    }
    #[test]
    fn destructive_requires_confirm() {
        // Act & Assert
        assert_eq!(
            verdict("ALTER TABLE t DROP COLUMN a", None),
            Verdict::Confirm
        );
    }
    #[test]
    fn truncate_big_table_requires_password() {
        // Act & Assert
        assert_eq!(verdict("TRUNCATE t", Some(5000)), Verdict::Password);
    }
    #[test]
    fn truncate_unknown_estimate_requires_password() {
        // Act & Assert
        assert_eq!(verdict("TRUNCATE t", None), Verdict::Password);
    }
    #[test]
    fn truncate_small_table_requires_confirm() {
        // Act & Assert
        assert_eq!(verdict("TRUNCATE t", Some(5)), Verdict::Confirm);
    }
    #[test]
    fn copy_from_requires_password() {
        // Arrange
        let sql = "COPY t FROM '/tmp/x.csv'";

        // Act & Assert
        assert_eq!(
            estimate_need(&classify(sql, DriverKind::Postgres)),
            EstimateNeed::None
        );
        assert_eq!(verdict(sql, None), Verdict::Password);
    }
    #[test]
    fn unrecognized_requires_confirm() {
        // Act & Assert
        assert_eq!(verdict("CALL p()", None), Verdict::Confirm);
    }
    #[test]
    fn batch_takes_strictest_verdict() {
        // Act
        let e = eval_batch(
            &[
                ("SELECT 1", None),
                ("DELETE FROM a", Some(3)),
                ("UPDATE b SET x=1 WHERE y>0", Some(5000)),
            ],
            1000,
        );

        // Assert
        assert_eq!(e.verdict, Verdict::Password);
        assert_eq!(e.flags.len(), 2);
    }
    #[test]
    fn reads_never_flagged() {
        // Act & Assert
        assert!(eval("SELECT * FROM t", None, 1).flags.is_empty());
    }
}
