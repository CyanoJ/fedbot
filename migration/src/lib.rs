pub use sea_orm_migration::prelude::*;

mod m20230424_115243_entry_modals;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20230424_115243_entry_modals::Migration),
        ]
    }
}
