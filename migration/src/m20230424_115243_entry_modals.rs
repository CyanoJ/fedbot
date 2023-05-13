use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Servers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Servers::Id)
                            .big_unsigned() 
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Servers::RulesChannel).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::ScreeningChannel).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::QuestioningRole).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::QuestioningCategory).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::ModRole).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::ModChannel).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::MemberRole).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::MainChannel).big_unsigned().not_null())
                    .col(ColumnDef::new(Servers::BlockedImages).blob(BlobSize::Tiny))
                    .col(ColumnDef::new(Servers::Triggers).blob(BlobSize::Medium))
                    .col(ColumnDef::new(Servers::EntryModal).blob(BlobSize::Medium))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Servers::Table).to_owned())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
enum Servers {
    Table,
    Id,
    RulesChannel,
    ScreeningChannel,
    QuestioningRole,
    QuestioningCategory,
    ModRole,
    ModChannel,
    MemberRole,
    MainChannel,
    BlockedImages,
    Triggers,
    EntryModal
}
