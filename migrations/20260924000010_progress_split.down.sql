-- Volta temporada e influência (só a semana mais recente) para o JSON.
UPDATE characters SET progress = progress || jsonb_build_object(
    'season', season, 'season_points', season_points, 'crowns', crowns);
UPDATE characters c SET progress = c.progress || jsonb_build_object(
    'influence_week', w.week, 'influence', w.ports)
FROM (
    SELECT DISTINCT ON (character_id) character_id, week,
        (SELECT jsonb_object_agg(port, points) FROM port_influence p
          WHERE p.character_id = l.character_id AND p.week = l.week) AS ports
    FROM port_influence l
    ORDER BY character_id, week DESC
) w
WHERE c.id = w.character_id;
DROP TABLE port_influence;
DROP INDEX idx_characters_season_points;
ALTER TABLE characters DROP COLUMN season, DROP COLUMN season_points, DROP COLUMN crowns;
