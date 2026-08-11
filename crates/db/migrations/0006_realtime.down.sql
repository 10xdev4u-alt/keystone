-- 0006_realtime.down.sql — reverse of the Month 7 schema
DROP TABLE IF EXISTS presence;
DROP TABLE IF EXISTS messages;
DROP TABLE IF EXISTS conversation_members;
DROP TABLE IF EXISTS conversations;
DROP TABLE IF EXISTS notification_deliveries;
DROP TABLE IF EXISTS notification_states;
DROP TABLE IF EXISTS notifications;
DROP TABLE IF EXISTS notification_preferences;
