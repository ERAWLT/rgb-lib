pub use sea_orm_migration::prelude::*;

mod m20230608_071249_init_db;
mod m20251017_074408_asset_update;
mod m20251105_132121_asset_update;
mod m20251215_124959_backup_info_update;
// UTEXO-Protocol/rgb-lib only (v0.3.0-beta.34): kept so that a database the ERA fork wrote on
// that base, which has them applied, opens here (see ERA.md, "Carried onto beta.8")
mod m20260401_000001_create_mpc_address_table;
mod m20260414_134758_add_reserved_txo;
mod m20260415_000001_create_reuse_address_index_table;
mod m20260625_121819_incoming_rework;
mod m20260727_115821_add_bdk_tables;
mod m20261019_120000_add_transfer_indexes;
mod m20261020_093003_add_pending_witness_indexes;
mod m20261020_111428_reserved_txo_colored;
mod m20261020_141856_add_rgb_storage_tables;
mod m20261020_142324_proxy_v1;
mod m20261020_142840_asset_transfer_transition;
mod m20261020_143313_asset_links;
mod m20261117_104500_asset_reject_list;
mod m20261117_160000_asset_bridge_location;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20230608_071249_init_db::Migration),
            Box::new(m20251017_074408_asset_update::Migration),
            Box::new(m20251105_132121_asset_update::Migration),
            Box::new(m20251215_124959_backup_info_update::Migration),
            Box::new(m20260401_000001_create_mpc_address_table::Migration),
            Box::new(m20260414_134758_add_reserved_txo::Migration),
            Box::new(m20260415_000001_create_reuse_address_index_table::Migration),
            Box::new(m20260625_121819_incoming_rework::Migration),
            Box::new(m20260727_115821_add_bdk_tables::Migration),
            Box::new(m20261019_120000_add_transfer_indexes::Migration),
            Box::new(m20261020_093003_add_pending_witness_indexes::Migration),
            Box::new(m20261020_111428_reserved_txo_colored::Migration),
            Box::new(m20261020_141856_add_rgb_storage_tables::Migration),
            Box::new(m20261020_142324_proxy_v1::Migration),
            Box::new(m20261020_142840_asset_transfer_transition::Migration),
            Box::new(m20261020_143313_asset_links::Migration),
            Box::new(m20261117_104500_asset_reject_list::Migration),
            Box::new(m20261117_160000_asset_bridge_location::Migration),
        ]
    }
}
