use sqlx_core::{placeholders, Execute, Result, Runtime};

use crate::protocol::frontend::{self, Bind, PortalRef, Query, StatementRef, Sync};
use crate::raw_statement::RawStatement;
use crate::{PgArguments, PgConnection, Postgres};
use std::borrow::Cow;

impl<Rt: Runtime> PgConnection<Rt> {
    fn write_raw_query_statement(
        &mut self,
        statement: &RawStatement,
        arguments: &PgArguments<'_>,
    ) -> Result<()> {
        // bind values to the prepared statement
        self.stream.write_message(&Bind {
            portal: PortalRef::Unnamed,
            statement: StatementRef::Named(statement.id),
            arguments,
            parameters: &statement.parameters,
        })?;

        // describe the bound prepared statement (portal)
        self.stream.write_message(&frontend::Describe {
            target: frontend::Target::Portal(PortalRef::Unnamed),
        })?;

        // execute the bound prepared statement (portal)
        self.stream
            .write_message(&frontend::Execute { portal: PortalRef::Unnamed, max_rows: 0 })?;

        // <Sync> is what closes the extended query invocation and
        // issues a <ReadyForQuery>
        self.stream.write_message(&Sync)?;

        Ok(())
    }
}

macro_rules! impl_raw_query {
    ($(@$blocking:ident)? $self:ident, $query:ident) => {{
        if let Some(arguments) = $query.arguments() {
            let statement = raw_prepare!($(@$blocking)? $self, $query.sql(), arguments);

            $self.write_raw_query_statement(&statement, arguments)?;
        } else {
            $self.stream.write_message(&Query { sql: $query.sql() })?;
        };

        // as we have written a SQL command of some kind to the stream
        // we now expect there to be an eventual ReadyForQuery
        // if for some reason the future for one of the execution methods is dropped
        // half-way through, we need to flush the stream until the ReadyForQuery point
        $self.pending_ready_for_query_count += 1;

        Ok(())
    }};
}

impl<Rt: Runtime> PgConnection<Rt> {
    #[cfg(feature = "async")]
    pub(super) async fn raw_query_async<'q, 'a, E>(&mut self, query: E) -> Result<()>
    where
        Rt: sqlx_core::Async,
        E: Execute<'q, 'a, Postgres>,
    {
        flush!(self);
        impl_raw_query!(self, query)
    }

    #[cfg(feature = "blocking")]
    pub(super) fn raw_query_blocking<'q, 'a, E>(&mut self, query: E) -> Result<()>
    where
        Rt: sqlx_core::blocking::Runtime,
        E: Execute<'q, 'a, Postgres>,
    {
        flush!(@blocking self);
        impl_raw_query!(@blocking self, query)
    }
}

macro_rules! raw_query {
    (@blocking $self:ident, $sql:expr) => {
        $self.raw_query_blocking($sql)?
    };

    ($self:ident, $sql:expr) => {
        $self.raw_query_async($sql).await?
    };
}
