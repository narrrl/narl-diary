-- A device's root folder rejects files: Proton answers 422 "Cannot create file
-- at the root of a device". Everything therefore moved one level down, into a
-- `data/` folder, which changed every remote path. The bookkeeping describes a
-- layout that no longer exists, so it is dropped and the mirror rebuilds itself
-- on the next run — an upload again, not a loss.

DELETE FROM backup_files;
DELETE FROM backup_state;
