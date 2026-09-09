-- Shared links gain a secret that rides in the URL fragment: /s/<token>#<key>.
-- Browsers never send a fragment to the origin, so a crawler that harvests the
-- link's path alone still cannot read the entry. The server keeps only the
-- key's hash; the plaintext exists once, in the link the author copied.
--
-- Links minted before this migration have no key, and there is no way to give
-- them one without leaving a permanent keyless bypass, so they are revoked.
ALTER TABLE entries ADD COLUMN share_key_hash TEXT;

UPDATE entries SET share_token = NULL WHERE share_token IS NOT NULL;
