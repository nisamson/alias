-- Your SQL goes here

create table users
(
    id       integer not null primary key autoincrement,
    username text    not null unique,
    hash     text    not null
);

create table aliases
(
    alias       text    not null primary key,
    destination text    not null,
    creator     integer not null,
    foreign key (creator)
        references users (id)
        on delete cascade
);

create index alias_creators on aliases(creator);
create unique index usernames on users(username);