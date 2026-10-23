//! ER diagram lookup on the Metadata session.
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{DriverKind, ObjectKind};

use super::{NO_TAB, WorkspaceActor, not_connected};
use crate::runtime::{DiagramTable, Event, RequestId};

impl WorkspaceActor {
    /// Tables of a schema with their columns and FKs, for the ER diagram.
    pub(super) async fn load_diagram(
        &mut self,
        req: RequestId,
        database: String,
        schema: Option<String>,
    ) {
        let tables = match self.diagram(&database, schema.as_deref()).await {
            Ok(t) => Some(t),
            Err((key, e)) => {
                if matches!(e.kind, ErrorKind::Connect | ErrorKind::Protocol) {
                    self.mark_down(key);
                }
                let text = e.message;
                self.out.emit(Event::Notice { tab: NO_TAB, text });
                None
            }
        };
        self.out.emit(Event::Diagram { req, tables });
    }

    async fn diagram(
        &mut self,
        database: &str,
        schema: Option<&str>,
    ) -> Result<Vec<DiagramTable>, (String, DriverError)> {
        let Some(sh) = self.shared.clone() else {
            return Err((String::new(), not_connected()));
        };
        let key = if sh.driver.kind() == DriverKind::Postgres {
            database.to_string()
        } else {
            self.home.clone()
        };
        let err = |e| (key.clone(), e);
        self.ensure_metadata(&key).await.map_err(err)?;
        let s = self
            .sessions
            .get_mut(&key)
            .ok_or_else(|| err(not_connected()))?;
        let intro = sh.driver.introspector();
        let objects = intro
            .objects(s.as_mut(), database, schema, ObjectKind::Table)
            .await
            .map_err(err)?;
        // ponytail: N+1 queries per table, single bulk catalog query if schemas get large
        let mut tables = Vec::with_capacity(objects.len());
        for object in objects {
            let columns = intro.columns(s.as_mut(), &object).await.map_err(err)?;
            let fks = intro.foreign_keys(s.as_mut(), &object).await.map_err(err)?;
            let columns = columns.into_iter().map(|c| c.meta).collect();
            tables.push(DiagramTable {
                object,
                columns,
                fks,
            });
        }
        Ok(tables)
    }
}
