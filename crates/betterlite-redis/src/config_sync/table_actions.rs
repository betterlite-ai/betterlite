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

pub struct ConfigSyncTarget<F> {
    pub object_type: &'static str,
    pub publish: PublishChange<F>,
}

pub struct ConfigSyncTableActions<A>
where
    A: TableActions,
{
    pub actions: A,
    pub sync: Option<ConfigSyncTarget<A::Failure>>,
}

impl<A> ConfigSyncTableActions<A>
where
    A: TableActions,
{
    async fn finish<T, F>(&self, action: F) -> Result<T, A::Failure>
    where
        T: Send,
        F: Future<Output = Result<T, A::Failure>> + Send,
    {
        let value = action.await?;
        if let Some(sync) = &self.sync {
            (sync.publish)(sync.object_type).await?;
        }
        Ok(value)
    }
}

impl<A> TableActions for ConfigSyncTableActions<A>
where
    A: TableActions,
{
    type Row = A::Row;
    type Filter = A::Filter;
    type CreateData = A::CreateData;
    type UpdateData = A::UpdateData;
    type Failure = A::Failure;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        self.finish(self.actions.create(data)).await
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        self.finish(self.actions.create_many(data)).await
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        self.finish(self.actions.update(data, filter)).await
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        self.finish(self.actions.update_many(data, filter)).await
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        self.finish(self.actions.upsert(filter, data)).await
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        self.finish(self.actions.delete(filter)).await
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        self.finish(self.actions.delete_many(filter)).await
    }
}

impl<A> AsRef<A> for ConfigSyncTableActions<A>
where
    A: TableActions,
{
    fn as_ref(&self) -> &A {
        &self.actions
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn wrap_table_actions_for_config_sync<A>(
    actions: A,
    table_name: &str,
    publish: PublishChange<A::Failure>,
) -> ConfigSyncTableActions<A>
where
    A: TableActions,
{
    let sync = SyncedTable::ALL
        .iter()
        .find(|table| table.name() == table_name)
        .map(|table| ConfigSyncTarget {
            object_type: table.name(),
            publish,
        });
    ConfigSyncTableActions { actions, sync }
}
