#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![cfg_attr(feature = "fail-on-warnings", deny(clippy::all))]

mod config;
pub mod custodian;
pub mod error;
mod primitives;

use tracing::instrument;

use audit::AuditSvc;
use authz::PermissionCheck;

pub use custodian::*;

pub use config::*;
use error::CoreCustodyError;
pub use primitives::*;

#[cfg(feature = "json-schema")]
pub mod event_schema {
    pub use crate::custodian::CustodianEvent;
}

#[derive(Clone)]
pub struct CoreCustody<Perms>
where
    Perms: PermissionCheck,
{
    authz: Perms,
    custodians: CustodianRepo,
    config: CustodyConfig,
}

impl<Perms> CoreCustody<Perms>
where
    Perms: PermissionCheck,
    <<Perms as PermissionCheck>::Audit as AuditSvc>::Action: From<CoreCustodyAction>,
    <<Perms as PermissionCheck>::Audit as AuditSvc>::Object: From<CoreCustodyObject>,
{
    pub async fn init(
        pool: &sqlx::PgPool,
        authz: &Perms,
        config: CustodyConfig,
    ) -> Result<Self, CoreCustodyError> {
        let custody = Self {
            authz: authz.clone(),
            custodians: CustodianRepo::new(pool),
            config,
        };

        if let Some(deprecated_encryption_key) = custody.config.deprecated_encryption_key.as_ref() {
            custody
                .rotate_encryption_key(deprecated_encryption_key)
                .await?;
        }

        Ok(custody)
    }

    #[instrument(
        name = "core_custody.create_custodian",
        skip(self, custodian_config),
        err
    )]
    pub async fn create_custodian(
        &self,
        sub: &<<Perms as PermissionCheck>::Audit as AuditSvc>::Subject,
        name: impl AsRef<str> + std::fmt::Debug,
        custodian_config: CustodianConfig,
    ) -> Result<Custodian, CoreCustodyError> {
        let audit_info = self
            .authz
            .enforce_permission(
                sub,
                CoreCustodyObject::all_custodians(),
                CoreCustodyAction::CUSTODIAN_CREATE,
            )
            .await?;

        let new_custodian = NewCustodian::builder()
            .id(CustodianId::new())
            .name(name.as_ref().to_owned())
            .audit_info(audit_info.clone())
            .encrypted_custodian_config(custodian_config, &self.config.custodian_encryption.key)
            .build()
            .expect("should always build a new custodian");

        let mut op = self.custodians.begin_op().await?;

        let custodian = self.custodians.create_in_op(&mut op, new_custodian).await?;

        op.commit().await?;

        Ok(custodian)
    }

    pub async fn update_config(
        &self,
        sub: &<<Perms as PermissionCheck>::Audit as AuditSvc>::Subject,
        custodian_id: impl Into<CustodianId> + std::fmt::Debug,
        config: CustodianConfig,
    ) -> Result<Custodian, CoreCustodyError> {
        let id = custodian_id.into();
        let audit_info = self
            .authz
            .enforce_permission(
                sub,
                CoreCustodyObject::custodian(id),
                CoreCustodyAction::CUSTODIAN_UPDATE,
            )
            .await?;
        let mut custodian = self.custodians.find_by_id(id).await?;

        custodian.update_custodian_config(
            config,
            &self.config.custodian_encryption.key,
            audit_info,
        );

        let mut op = self.custodians.begin_op().await?;
        self.custodians
            .update_config_in_op(&mut op, &mut custodian)
            .await?;
        op.commit().await?;

        Ok(custodian)
    }

    async fn rotate_encryption_key(
        &self,
        deprecated_encryption_key: &DeprecatedEncryptionKey,
    ) -> Result<(), CoreCustodyError> {
        let audit_info = self
            .authz
            .audit()
            .record_system_entry(
                CoreCustodyObject::all_custodians(),
                CoreCustodyAction::CUSTODIAN_UPDATE,
            )
            .await?;

        let mut custodians = self.custodians.list_all().await?;

        let mut op = self.custodians.begin_op().await?;

        for custodian in custodians.iter_mut() {
            custodian.rotate_encryption_key(
                &self.config.custodian_encryption.key,
                deprecated_encryption_key,
                &audit_info,
            )?;

            self.custodians
                .update_config_in_op(&mut op, custodian)
                .await?;
        }

        op.commit().await?;

        Ok(())
    }

    #[instrument(name = "core_custody.find_all_custodians", skip(self), err)]
    pub async fn find_all_custodians<T: From<Custodian>>(
        &self,
        ids: &[CustodianId],
    ) -> Result<std::collections::HashMap<CustodianId, T>, CoreCustodyError> {
        Ok(self.custodians.find_all(ids).await?)
    }

    #[instrument(name = "core_custody.list_custodians", skip(self), err)]
    pub async fn list_custodians(
        &self,
        sub: &<<Perms as PermissionCheck>::Audit as AuditSvc>::Subject,
        query: es_entity::PaginatedQueryArgs<CustodiansByNameCursor>,
    ) -> Result<es_entity::PaginatedQueryRet<Custodian, CustodiansByNameCursor>, CoreCustodyError>
    {
        self.authz
            .enforce_permission(
                sub,
                CoreCustodyObject::all_custodians(),
                CoreCustodyAction::CUSTODIAN_LIST,
            )
            .await?;
        Ok(self
            .custodians
            .list_by_name(query, es_entity::ListDirection::Ascending)
            .await?)
    }
}
