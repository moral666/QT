//! Copias de recuperacao nos dois relays (e o que e preciso para uma conta recuperada noutro
//! relay voltar a casa sem perder mensagens).
//!
//! Cada conta tem em cada relay um destes estados: uma copia, uma marca de "apagada" ou nada.
//! `bkat` diz quando mudou pela ultima vez; entre relays fica sempre a alteracao mais recente,
//! por isso uma copia velha nunca passa por cima de uma nova nem volta depois de apagada.
//! O relay onde a conta esta poe-na na fila de cada par (`bksync`); a federacao esvazia a fila
//! quando o par responde. Chaves:
//!   {p}:bkat:{conta}       ms da ultima alteracao da copia (gravada, apagada ou banida)
//!   {p}:bksync:{par}       contas cuja copia ainda falta confirmar nesse par
//!   {p}:fed:movida:{conta} a conta estava aqui por verificar e afinal e do relay indicado
//!   {p}:fed:encaminhar     contas com mensagens aqui para passar ao relay delas

use crate::store::{now_ms, Store, StoreError, BACKUP_TTL_SECONDS};
use redis::AsyncCommands;

/// Estado da copia de uma conta neste relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoCopia {
    /// Ultima alteracao (ms); 0: copia anterior a esta versao, ou nunca houve nada.
    pub at: u64,
    pub copia: Option<CopiaAtual>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiaAtual {
    pub lookup: String,
    pub generation: String,
    pub parts: u32,
    /// Segundos que ainda dura.
    pub ttl: i64,
}

impl Store {
    pub(crate) fn bk_at_key(&self, owner: &str) -> String {
        format!("{}:bkat:{owner}", self.key_prefix)
    }

    /// Alteracao feita neste relay: hora nova, sempre maior que a anterior (os relogios dos
    /// relays podem diferir uns milissegundos).
    pub(crate) async fn bk_bump(&self, owner: &str, ttl: i64) -> Result<u64, StoreError> {
        let mut conn = self.connection().await?;
        let key = self.bk_at_key(owner);
        let old: Option<u64> = conn.get(&key).await?;
        let at = now_ms().max(old.unwrap_or(0).saturating_add(1));
        self.bk_set_at(owner, at, ttl).await?;
        Ok(at)
    }

    pub(crate) async fn bk_set_at(&self, owner: &str, at: u64, ttl: i64) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = self.bk_at_key(owner);
        redis::pipe().set(&key, at).ignore().expire(&key, ttl.max(1)).ignore().query_async::<_, ()>(&mut conn).await?;
        Ok(())
    }

    pub async fn backup_state(&self, owner: &str) -> Result<EstadoCopia, StoreError> {
        let mut conn = self.connection().await?;
        let (at, lookup): (Option<u64>, Option<String>) = redis::pipe()
            .get(self.bk_at_key(owner))
            .get(format!("{}:bkof:{owner}", self.key_prefix))
            .query_async(&mut conn)
            .await?;
        let mut copia = None;
        if let Some(lookup) = lookup {
            if let Some((generation, parts)) = self.bk_head(&mut conn, &lookup).await? {
                let ttl: i64 = conn.ttl(self.bk_keys(&lookup).0).await?;
                if ttl > 0 {
                    copia = Some(CopiaAtual { lookup, generation, parts, ttl });
                }
            }
        }
        Ok(EstadoCopia { at: at.unwrap_or(0), copia })
    }

    /// Todas as partes de uma geracao. `None`: entretanto mudou (a conta volta a fila).
    pub async fn backup_all_parts(&self, copia: &CopiaAtual) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let mut out = Vec::with_capacity(copia.parts as usize);
        for i in 0..copia.parts {
            match self.backup_get(&copia.lookup, i).await? {
                Some((n, data)) if n == copia.parts => out.push(data),
                _ => return Ok(None),
            }
        }
        let mut conn = self.connection().await?;
        let same = self.bk_head(&mut conn, &copia.lookup).await?.is_some_and(|(g, _)| g == copia.generation);
        Ok(same.then_some(out))
    }

    /// Outro relay tem esta geracao desde `at`, por mais `ttl` segundos. Verdadeiro: faltam os
    /// dados. Falso: ja a temos (fica com o prazo certo) ou temos algo mais recente.
    pub async fn replica_check(
        &self,
        owner: &str,
        lookup: &str,
        generation: &str,
        at: u64,
        ttl: i64,
    ) -> Result<bool, StoreError> {
        let local = self.backup_state(owner).await?;
        if local.at > at {
            return Ok(false);
        }
        match &local.copia {
            Some(c) if c.lookup == lookup && c.generation == generation => {
                // Alteracao mais recente (ex.: banida): o prazo dela. A mesma: o maior.
                let ttl = if at > local.at { ttl } else { ttl.max(c.ttl) };
                self.backup_expire_in(owner, lookup, ttl).await?;
                self.bk_set_at(owner, at, ttl).await?;
                Ok(false)
            }
            // Apagada aqui ao mesmo tempo ou depois.
            None if at != 0 && local.at == at => Ok(false),
            _ => Ok(true),
        }
    }

    /// Apagada noutro relay em `at`.
    pub async fn replica_delete(&self, owner: &str, at: u64) -> Result<(), StoreError> {
        if self.backup_state(owner).await?.at > at {
            return Ok(());
        }
        // A marca primeiro: uma copia mais velha que chegue entretanto ja nao entra.
        self.bk_set_at(owner, at, BACKUP_TTL_SECONDS).await?;
        self.delete_backup_files(owner).await
    }

    // ----- Fila de cada par -----

    fn bk_sync_key(&self, par: &str) -> String {
        format!("{}:bksync:{par}", self.key_prefix)
    }

    /// A copia desta conta tem de ser confirmada em `par` (uma so vez na fila).
    pub async fn bk_sync_add(&self, par: &str, owner: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let key = self.bk_sync_key(par);
        redis::pipe().atomic().lrem(&key, 0, owner).ignore().rpush(&key, owner).ignore().query_async::<_, ()>(&mut conn).await?;
        Ok(())
    }

    pub async fn bk_sync_take(&self, par: &str) -> Result<Option<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(redis::cmd("LPOP").arg(self.bk_sync_key(par)).query_async(&mut conn).await?)
    }

    pub async fn bk_sync_len(&self, par: &str) -> Result<usize, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.llen(self.bk_sync_key(par)).await?)
    }

    /// Contas com copia num ficheiro cujo nome comeca por `prefixo` (dois hex).
    pub async fn backup_owners(&self, prefixo: u8) -> Result<Vec<String>, StoreError> {
        let Some(backups) = &self.backups else { return Ok(Vec::new()) };
        let lookups = backups.list(prefixo).await?;
        let mut conn = self.connection().await?;
        let mut out = Vec::with_capacity(lookups.len());
        for lookup in lookups {
            if let Some(owner) = conn.get::<_, Option<String>>(self.bk_keys(&lookup).1).await? {
                out.push(owner);
            }
        }
        Ok(out)
    }

    // ----- Contas que voltam ao relay delas -----

    pub(crate) fn fed_movida_key(&self, user_id: &str) -> String {
        format!("{}:fed:movida:{user_id}", self.key_prefix)
    }

    fn fed_encaminhar_key(&self) -> String {
        format!("{}:fed:encaminhar", self.key_prefix)
    }

    /// A conta afinal e do relay `casa`: guarda isso e poe as mensagens dela por encaminhar.
    pub async fn fed_marcar_movida(&self, user_id: &str, casa: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        let (key, list) = (self.fed_movida_key(user_id), self.fed_encaminhar_key());
        redis::pipe()
            .set(&key, casa)
            .ignore()
            .expire(&key, crate::store::BUNDLE_TTL_SECONDS as i64)
            .ignore()
            .lrem(&list, 0, user_id)
            .ignore()
            .rpush(&list, user_id)
            .ignore()
            .query_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn fed_movida(&self, user_id: &str) -> Result<Option<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.get(self.fed_movida_key(user_id)).await?)
    }

    pub async fn fed_por_encaminhar(&self, max: usize) -> Result<Vec<String>, StoreError> {
        let mut conn = self.connection().await?;
        Ok(conn.lrange(self.fed_encaminhar_key(), 0, max.max(1) as isize - 1).await?)
    }

    pub async fn fed_encaminhada(&self, user_id: &str) -> Result<(), StoreError> {
        let mut conn = self.connection().await?;
        conn.lrem::<_, _, ()>(self.fed_encaminhar_key(), 0, user_id).await?;
        Ok(())
    }
}
