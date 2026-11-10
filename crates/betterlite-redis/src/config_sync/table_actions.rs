use std::future::Future;

use super::publish::PublishChange;
use super::tables::SyncedTable;

pub trait TableActions: Send + Sync {
    type Row: Send + 'static;
    type Filter: Send + 'static;
    type CreateData: Send + 'static;
    type UpdateData: Send + 'static;
    type Failure: Send + 'static;

    fn create(
        &self,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send;

    fn create_many(
        &self,
        data: Vec<Self::CreateData>,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;

    fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send;

    fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;

    fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send;

    fn delete(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send;

    fn delete_many(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;
}

pub struct WrappedTableActions<A>
where
    A: TableActions,
{
    pub actions: A,
    pub object_type: &'static str,
    pub publish: PublishChange<A::Failure>,
}

impl<A> TableActions for WrappedTableActions<A>
where
    A: TableActions,
{
    type Row = A::Row;
    type Filter = A::Filter;
    type CreateData = A::CreateData;
    type UpdateData = A::UpdateData;
    type Failure = A::Failure;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        let row = self.actions.create(data).await?;
        (self.publish)(self.object_type).await?;
        Ok(row)
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        let written = self.actions.create_many(data).await?;
        (self.publish)(self.object_type).await?;
        Ok(written)
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        let row = self.actions.update(data, filter).await?;
        (self.publish)(self.object_type).await?;
        Ok(row)
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        let written = self.actions.update_many(data, filter).await?;
        (self.publish)(self.object_type).await?;
        Ok(written)
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        let row = self.actions.upsert(filter, data).await?;
        (self.publish)(self.object_type).await?;
        Ok(row)
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        let row = self.actions.delete(filter).await?;
        (self.publish)(self.object_type).await?;
        Ok(row)
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        let written = self.actions.delete_many(filter).await?;
        (self.publish)(self.object_type).await?;
        Ok(written)
    }
}

pub enum SyncedActions<A>
where
    A: TableActions,
{
    Unwrapped(A),
    Wrapped(WrappedTableActions<A>),
}

impl<A> TableActions for SyncedActions<A>
where
    A: TableActions,
{
    type Row = A::Row;
    type Filter = A::Filter;
    type CreateData = A::CreateData;
    type UpdateData = A::UpdateData;
    type Failure = A::Failure;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.create(data).await,
            Self::Wrapped(wrapped) => wrapped.create(data).await,
        }
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.create_many(data).await,
            Self::Wrapped(wrapped) => wrapped.create_many(data).await,
        }
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.update(data, filter).await,
            Self::Wrapped(wrapped) => wrapped.update(data, filter).await,
        }
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.update_many(data, filter).await,
            Self::Wrapped(wrapped) => wrapped.update_many(data, filter).await,
        }
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.upsert(filter, data).await,
            Self::Wrapped(wrapped) => wrapped.upsert(filter, data).await,
        }
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.delete(filter).await,
            Self::Wrapped(wrapped) => wrapped.delete(filter).await,
        }
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        match self {
            Self::Unwrapped(actions) => actions.delete_many(filter).await,
            Self::Wrapped(wrapped) => wrapped.delete_many(filter).await,
        }
    }
}

impl<A> AsRef<A> for SyncedActions<A>
where
    A: TableActions,
{
    fn as_ref(&self) -> &A {
        match self {
            Self::Unwrapped(actions) => actions,
            Self::Wrapped(wrapped) => &wrapped.actions,
        }
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn wrap_table_actions_for_config_sync<A>(
    actions: A,
    table_name: &str,
    publish: PublishChange<A::Failure>,
) -> SyncedActions<A>
where
    A: TableActions,
{
    let synced = SyncedTable::ALL
        .iter()
        .find(|table| table.name() == table_name);
    match synced {
        Some(table) => SyncedActions::Wrapped(WrappedTableActions {
            actions,
            object_type: table.name(),
            publish,
        }),
        None => SyncedActions::Unwrapped(actions),
    }
}
