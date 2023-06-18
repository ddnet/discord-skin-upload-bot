use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::sync::Arc;
use std::time::Duration;

use hashlink::LinkedHashMap;
use image::{ColorType, ImageFormat};
use serenity::all::{GuildId, Interaction, Mention, MessageId, Reaction, Ready, RoleId, UserId};
use serenity::async_trait;
use serenity::builder::{
    CreateAllowedMentions, CreateCommand, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, EditInteractionResponse,
};
use serenity::framework::standard::StandardFramework;
use serenity::prelude::*;
use tokio::select;
use tokio::sync::Notify;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            let guild_id = GuildId::new(
                env::var("GUILD_ID")
                    .expect("Expected GUILD_ID in environment")
                    .parse()
                    .expect("GUILD_ID must be an integer"),
            );
            if command
                .user
                .has_role(
                    ctx.clone(),
                    guild_id,
                    RoleId::new(
                        env::var("ROLE_ID")
                            .expect("Expected ROLE_ID in environment")
                            .parse()
                            .expect("ROLE_ID must be an integer"),
                    ),
                )
                .await
                .unwrap_or(false)
            {
                let database_url =
                    env::var("DATABASE_URL").unwrap_or("https://ddnet.org/skins/".to_string());
                let basic_auth_user_name =
                    env::var("USERNAME").expect("Expected USERNAME for http auth in environment");
                let basic_auth_password =
                    env::var("PASSWORD").expect("Expected PASSWORD for http auth in environment");
                let main_cmd_str = "\
                    You are about to upload a skin to the database.\n\
                    Please react to all skins you want to upload:\n\
                    - React with ✅ to upload a skin to the normal database\n\
                    - React with ☑️ to upload a skin to the community database\n\
                    "
                .to_string();
                let main_cmd_end_str = "\
                    Once you are done, use the command /upload_finish\n\
                    If you want to cancel the upload, use /upload_cancel\n\
                    "
                .to_string();
                let content = match command.data.name.as_str() {
                    "upload" => Some(main_cmd_str.clone() + &main_cmd_end_str),
                    "upload_finish" => {
                        let mut data = ctx.data.write().await;
                        if let Some(item) = data
                            .get_mut::<SkinUploads>()
                            .unwrap()
                            .uploads
                            .get_mut(&command.user.id)
                        {
                            if let SkinUploadState::Collecting = item.state {
                                item.state = SkinUploadState::Uploading;
                                item.notify.notify_one();

                                // let's upload
                                let mut skins_to_upload = item.skins_to_upload.clone();
                                let upload_lock =
                                    data.get_mut::<SkinUploads>().unwrap().upload_lock.clone();
                                drop(data);

                                let _g = upload_lock.lock().await;
                                let data = CreateInteractionResponseMessage::new()
                                    .content("Starting to upload")
                                    .ephemeral(true);
                                let builder = CreateInteractionResponse::Message(data);
                                if let Err(why) = command.create_response(&ctx.http, builder).await
                                {
                                    println!("Could not respond to slash command: {why}");
                                }

                                let errors: Arc<Mutex<Vec<String>>> = Default::default();
                                let mut uploaded_skins_msg: String =
                                    "The following skins were added to the database:\n".to_string();
                                let mut uploaded_skin_users: HashSet<UserId> = Default::default();
                                for (skin_name, skin_to_upload) in skins_to_upload.drain() {
                                    let author = skin_to_upload.author;
                                    let license = skin_to_upload.license;
                                    let database = skin_to_upload.database.to_string();
                                    let get_form_base = Arc::new(move |img_name: String| {
                                        let mut form = reqwest::blocking::multipart::Form::new();
                                        form = form.file("image", img_name + ".png").unwrap();
                                        form = form.text("creator", author.clone());
                                        form = form.text("skin_pack", "");
                                        form = form.text("skin_license", license.clone());
                                        form = form.text("skin_type", database.clone());
                                        form = form.text("game_version", "tw-0.6");
                                        form = form.text("skin_part", "full");
                                        form = form.text("modifyaction", "add");
                                        form
                                    });

                                    if !skin_to_upload.file_256x128.is_empty() {
                                        let errors_clone = errors.clone();
                                        let skin_name_clone = skin_name.clone();
                                        let get_form_base_clone = get_form_base.clone();
                                        let basic_auth_user_name = basic_auth_user_name.clone();
                                        let basic_auth_password = basic_auth_password.clone();
                                        let db_url = database_url.clone();
                                        tokio::task::spawn_blocking(move || {
                                            image::save_buffer_with_format(
                                                skin_name_clone.clone() + ".png",
                                                &skin_to_upload.file_256x128,
                                                256,
                                                128,
                                                ColorType::Rgba8,
                                                ImageFormat::Png,
                                            )
                                            .unwrap();
                                            let form = get_form_base_clone(skin_name_clone.clone())
                                                .text("skinisuhd", "false");
                                            if let Err(err) = reqwest::blocking::Client::new()
                                                .post(db_url + "edit/modify_skin.php")
                                                .multipart(form)
                                                .basic_auth(basic_auth_user_name, Some(basic_auth_password))
                                                .send()
                                            {
                                                errors_clone.blocking_lock().push(format!("There was an error while uploading {}.\nPlease manually check if this broke the database", err));
                                            }
                                        }).await.unwrap();

                                        tokio::fs::remove_file(skin_name.clone() + ".png")
                                            .await
                                            .unwrap();
                                    }

                                    if !skin_to_upload.file_512x256.is_empty() {
                                        let errors_clone = errors.clone();
                                        let skin_name_clone = skin_name.clone();
                                        let basic_auth_user_name = basic_auth_user_name.clone();
                                        let basic_auth_password = basic_auth_password.clone();
                                        let db_url = database_url.clone();
                                        tokio::task::spawn_blocking(move || {
                                            image::save_buffer_with_format(
                                                skin_name_clone.clone() + ".png",
                                                &skin_to_upload.file_512x256,
                                                512,
                                                256,
                                                ColorType::Rgba8,
                                                ImageFormat::Png,
                                            )
                                            .unwrap();
                                            let form = get_form_base(skin_name_clone.clone())
                                                .text("skinisuhd", "true");
                                            if let Err(err) = reqwest::blocking::Client::new()
                                                .post(db_url + "edit/modify_skin.php")
                                                .multipart(form)
                                                .basic_auth(basic_auth_user_name, Some(basic_auth_password))
                                                .send()
                                            {
                                                errors_clone.blocking_lock().push(format!("There was an error while uploading {}.\nPlease manually check if this broke the database", err));
                                            }}
                                        ).await.unwrap();

                                        tokio::fs::remove_file(skin_name.clone() + ".png")
                                            .await
                                            .unwrap();
                                    }

                                    if let Ok(msg) = command
                                        .channel_id
                                        .message(&ctx, skin_to_upload.original_msg_id)
                                        .await
                                    {
                                        uploaded_skins_msg += &("- \"".to_string()
                                            + &skin_name
                                            + "\" ["
                                            + &skin_to_upload.database.to_string()
                                            + "] by "
                                            + &Mention::User(msg.author.id).to_string()
                                            + " ("
                                            + &format!(
                                                "https://discord.com/channels/{}/{}/{}",
                                                guild_id.0, command.channel_id.0, msg.id.0
                                            )
                                            + ") \n");
                                        uploaded_skin_users.insert(msg.author.id);
                                    }
                                }

                                if let Err(err) = command
                                    .channel_id
                                    .send_message(
                                        &ctx,
                                        CreateMessage::new()
                                            .allowed_mentions(
                                                CreateAllowedMentions::new()
                                                    .users(uploaded_skin_users),
                                            )
                                            .content(uploaded_skins_msg),
                                    )
                                    .await
                                {
                                    println!("sending global uploaded skins message failed {err}.");
                                }

                                let mut new_msg = String::default();
                                new_msg += "Upload of skins finished.\n";
                                if !errors.lock().await.is_empty() {
                                    new_msg += "But there were the following errors:\n";
                                    for err in errors.lock().await.iter() {
                                        new_msg += &(err.clone() + "\n");
                                    }
                                }
                                if let Err(err) = command
                                    .edit_response(
                                        &ctx,
                                        EditInteractionResponse::new().content(new_msg),
                                    )
                                    .await
                                {
                                    println!("Could edit responds of upload finish: {err}");
                                }
                            } else {
                                let data = CreateInteractionResponseMessage::new()
                                    .content(
                                        "Upload already in progress, wait for the previous to end",
                                    )
                                    .ephemeral(true);
                                let builder = CreateInteractionResponse::Message(data);
                                if let Err(why) = command.create_response(&ctx.http, builder).await
                                {
                                    println!("Could not respond to slash command: {why}");
                                }
                            }
                        } else {
                            let data = CreateInteractionResponseMessage::new()
                                .content("You never started an upload, please use /upload")
                                .ephemeral(true);
                            let builder = CreateInteractionResponse::Message(data);
                            if let Err(why) = command.create_response(&ctx.http, builder).await {
                                println!("Could not respond to slash command: {why}");
                            }
                        }
                        return;
                    }
                    "upload_cancel" => {
                        let mut data = ctx.data.write().await;
                        if let Some(item) = data
                            .get_mut::<SkinUploads>()
                            .unwrap()
                            .uploads
                            .get_mut(&command.user.id)
                        {
                            if let SkinUploadState::Collecting = item.state {
                                let data = CreateInteractionResponseMessage::new()
                                    .content("Skin upload cancelled")
                                    .ephemeral(true);
                                let builder = CreateInteractionResponse::Message(data);
                                if let Err(why) = command.create_response(&ctx.http, builder).await
                                {
                                    println!("Could not respond to slash command: {why}");
                                }
                                item.state = SkinUploadState::Cancelled;
                                item.notify.notify_one();
                            } else {
                                let data = CreateInteractionResponseMessage::new()
                                    .content("Cannot cancel upload at this point anymore")
                                    .ephemeral(true);
                                let builder = CreateInteractionResponse::Message(data);
                                if let Err(why) = command.create_response(&ctx.http, builder).await
                                {
                                    println!("Could not respond to slash command: {why}");
                                }
                            }
                        } else {
                            let data = CreateInteractionResponseMessage::new()
                                .content("You never started an upload using `/upload`.")
                                .ephemeral(true);
                            let builder = CreateInteractionResponse::Message(data);
                            if let Err(why) = command.create_response(&ctx.http, builder).await {
                                println!("Could not respond to slash command: {why}");
                            }
                        }
                        return;
                    }
                    _ => None,
                };

                if !ctx
                    .data
                    .read()
                    .await
                    .get::<SkinUploads>()
                    .unwrap()
                    .uploads
                    .is_empty()
                {
                    let data = CreateInteractionResponseMessage::new()
                        .content("Someone is already uploading skins. Please wait. If the upload disconnected, wait ~2 minutes, until the timeout kicks in.")
                        .ephemeral(true);
                    let builder = CreateInteractionResponse::Message(data);
                    if let Err(why) = command.create_response(&ctx.http, builder).await {
                        println!("Could not respond to slash command: {why}");
                    }
                    return;
                }

                if let Some(content) = content {
                    let data = CreateInteractionResponseMessage::new()
                        .content(content)
                        .ephemeral(true);
                    let builder = CreateInteractionResponse::Message(data);
                    if let Err(why) = command.create_response(&ctx.http, builder).await {
                        println!("Could not respond to slash command: {why}");
                    } else {
                        let notify = Arc::new(Notify::new());
                        ctx.data
                            .write()
                            .await
                            .get_mut::<SkinUploads>()
                            .unwrap()
                            .uploads
                            .insert(
                                command.user.id,
                                SkinUploadItem {
                                    notify: notify.clone(),
                                    reaction_list: Default::default(),
                                    skins_try_upload: Default::default(),
                                    state: SkinUploadState::Collecting,
                                    errors: Default::default(),
                                    skins_to_upload: Default::default(),
                                },
                            );

                        loop {
                            let was_notified = select! {
                                _ = tokio::time::sleep(Duration::from_secs(120)) => {false}
                                _ = notify.notified() => {true}
                            };

                            let mut data = ctx.data.write().await;
                            // if data is still there, tell that the process was cancelled
                            if let Some(item) = data
                                .get_mut::<SkinUploads>()
                                .unwrap()
                                .uploads
                                .get_mut(&command.user.id)
                            {
                                if was_notified {
                                    match item.state {
                                        SkinUploadState::Collecting => {
                                            // check if all skins are valid
                                            for (msg_id, msg_database) in
                                                item.skins_try_upload.drain()
                                            {
                                                if let Ok(skin_msg) = ctx
                                                    .http
                                                    .get_message(command.channel_id, msg_id)
                                                    .await
                                                {
                                                    let text = skin_msg.content;
                                                    let mut all_required_info = true;
                                                    let mut skin_name = String::default();
                                                    let mut author_name = String::default();
                                                    let mut license_name = String::default();
                                                    let matches_text =
                                                        regex::Regex::new("\"([A-Z0-9a-z_ ]+)\"")
                                                            .unwrap();
                                                    let caps = matches_text.captures(&text);
                                                    if caps.is_some()
                                                        && caps.as_ref().unwrap().len() > 0
                                                    {
                                                        skin_name = caps
                                                            .unwrap()
                                                            .get(1)
                                                            .unwrap()
                                                            .as_str()
                                                            .to_string();
                                                        if let Some(skin) =
                                                            item.skins_to_upload.get(&skin_name)
                                                        {
                                                            if skin.database != msg_database {
                                                                item.errors.push_back(format!(
                                                                    "you changed the database upload type of: {}. If you did a mistake cancel the upload and try again.",
                                                                    skin_name
                                                                ));
                                                                all_required_info = false;
                                                            }
                                                        }
                                                    } else {
                                                        item.errors.push_back(format!(
                                                            "name not found in msg: {}",
                                                            text
                                                        ));
                                                        all_required_info = false;
                                                    }
                                                    let matches_text = regex::Regex::new(
                                                        "by ([A-Z0-9a-z_ -]+) \\(",
                                                    )
                                                    .unwrap();
                                                    let caps = matches_text.captures(&text);
                                                    if caps.is_some()
                                                        && caps.as_ref().unwrap().len() > 0
                                                    {
                                                        author_name = caps
                                                            .unwrap()
                                                            .get(1)
                                                            .unwrap()
                                                            .as_str()
                                                            .to_string();
                                                    } else {
                                                        item.errors.push_back(format!(
                                                            "author not found in msg: {}",
                                                            text
                                                        ));
                                                        all_required_info = false;
                                                    }
                                                    let matches_text = regex::Regex::new(
                                                        "\\(([A-Z0-9a-z_ -]+)\\)",
                                                    )
                                                    .unwrap();
                                                    let caps = matches_text.captures(&text);
                                                    if caps.is_some()
                                                        && caps.as_ref().unwrap().len() > 0
                                                    {
                                                        license_name = caps
                                                            .unwrap()
                                                            .get(1)
                                                            .unwrap()
                                                            .as_str()
                                                            .to_string();
                                                    } else {
                                                        item.errors.push_back(format!(
                                                            "license not found in msg: {}",
                                                            text
                                                        ));
                                                        all_required_info = false;
                                                    }
                                                    if all_required_info {
                                                        for attachment in
                                                            skin_msg.attachments.iter()
                                                        {
                                                            if let Ok(file) =
                                                                attachment.download().await
                                                            {
                                                                if let Ok(img) =
                                                                    image::load_from_memory(&file)
                                                                {
                                                                    if let Some(img_rgba) =
                                                                        img.as_rgba8()
                                                                    {
                                                                        if img_rgba.dimensions()
                                                                            == (256, 128)
                                                                            || img_rgba.dimensions()
                                                                                == (512, 256)
                                                                        {
                                                                            if !item
                                                                                .skins_to_upload
                                                                                .contains_key(
                                                                                    &skin_name,
                                                                                )
                                                                            {
                                                                                item.skins_to_upload.insert(skin_name.clone(), SkinToUpload {
                                                                                    author: author_name.clone(),
                                                                                    license: license_name.clone(),
                                                                                    file_256x128: Vec::new(),
                                                                                    file_512x256: Vec::new(),
                                                                                    database: msg_database.clone(),
                                                                                    original_msg_id: msg_id,
                                                                                });
                                                                            }
                                                                            if img_rgba.dimensions()
                                                                                == (256, 128)
                                                                            {
                                                                                item.skins_to_upload
                                                                                .get_mut(&skin_name)
                                                                                .unwrap()
                                                                                .file_256x128 =
                                                                                img_rgba.to_vec();
                                                                            } else {
                                                                                item.skins_to_upload
                                                                                    .get_mut(&skin_name)
                                                                                    .unwrap()
                                                                                    .file_512x256 =
                                                                                    img_rgba.to_vec();
                                                                            }
                                                                        } else {
                                                                            item.errors.push_back(format!("skin: {} did not contain a valid 256x128 or 512x256 skin", skin_name.clone()));
                                                                        }
                                                                    } else {
                                                                        item.errors.push_back("One of the reacted messages contained an image file that could not be converted to RGBA...".to_string());
                                                                    }
                                                                } else {
                                                                    item.errors.push_back("One of the reacted messages contained an invalid image file...".to_string());
                                                                }
                                                            } else {
                                                                item.errors.push_back("One of the reacted messages did not contain a valid skin file...".to_string());
                                                            }
                                                        }

                                                        if skin_msg.attachments.is_empty() {
                                                            item.errors.push_back("No skin file attachments found in one of the messages you reacted to...".to_string());
                                                        }
                                                    }
                                                } else {
                                                    item.errors.push_back("One of the reacted messages was not found anymore...".to_string());
                                                }
                                            }
                                        }
                                        SkinUploadState::Uploading | SkinUploadState::Cancelled => {
                                            if let Err(_) = command.delete_response(&ctx).await {
                                                println!("Response not deleted.");
                                            }
                                            data.get_mut::<SkinUploads>()
                                                .unwrap()
                                                .uploads
                                                .remove(&command.user.id);
                                            break;
                                        }
                                    };

                                    // edit msg
                                    let mut new_msg = main_cmd_str.clone();
                                    if !item.errors.is_empty() {
                                        new_msg += "**Errors**:\n";
                                        item.errors.iter().for_each(|err| {
                                            new_msg += "- ";
                                            new_msg += &err;
                                            new_msg += "\n";
                                        });
                                    }
                                    if !item.skins_to_upload.is_empty() {
                                        new_msg += "Skins to upload:\n";
                                        item.skins_to_upload.iter().for_each(
                                            |(skin_name, skin)| {
                                                new_msg += "- ";
                                                if let SkinToUploadDB::Normal = &skin.database  {
                                                    new_msg += "✅ ";
                                                }
                                                else{
                                                    new_msg += "☑️ ";
                                                }
                                                new_msg += "\"";
                                                new_msg += skin_name;
                                                new_msg += "\" by ";
                                                new_msg += &skin.author;
                                                new_msg += " license: \"";
                                                new_msg += &skin.license;
                                                new_msg += &format!("\" (has 256x128 skin: {}, has 512x256 skin: {})", !skin.file_256x128.is_empty(), !skin.file_512x256.is_empty());
                                                new_msg += "\n";
                                            },
                                        );
                                    }
                                    new_msg += &main_cmd_end_str;
                                    if let Err(err) = command
                                        .edit_response(
                                            ctx.clone(),
                                            EditInteractionResponse::new().content(new_msg),
                                        )
                                        .await
                                    {
                                        println!("Could not edit response from command: {err}");
                                    }
                                } else {
                                    if let Err(err) = command
                                    .edit_response(
                                        ctx.clone(),
                                        EditInteractionResponse::new().content(
                                            "Upload timed out. Also only do one upload at a time",
                                        ),
                                    )
                                    .await
                                    {
                                        println!("Could not edit response from command: {err}");
                                    }
                                    data.get_mut::<SkinUploads>()
                                        .unwrap()
                                        .uploads
                                        .remove(&command.user.id);
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            } else {
                let data = CreateInteractionResponseMessage::new()
                    .content(
                        "You don't have the required permissions to use this command".to_string(),
                    )
                    .ephemeral(true);
                let builder = CreateInteractionResponse::Message(data);
                if let Err(why) = command.create_response(&ctx.http, builder).await {
                    println!("Could not respond to slash command: {why}");
                }
            }
        }
    }

    async fn reaction_add(&self, ctx: Context, add_reaction: Reaction) {
        if !add_reaction.user_id.is_some() {
            return;
        }
        if add_reaction.emoji.unicode_eq("✅") {
            if let Some(skin_upload) = ctx
                .clone()
                .data
                .write()
                .await
                .get_mut::<SkinUploads>()
                .unwrap()
                .uploads
                .get_mut(&add_reaction.user_id.unwrap())
            {
                skin_upload
                    .reaction_list
                    .insert(add_reaction.message_id, add_reaction.user_id.unwrap());
                skin_upload
                    .skins_try_upload
                    .insert(add_reaction.message_id, SkinToUploadDB::Normal);
                skin_upload.notify.notify_one();
            }
        } else {
            if add_reaction.emoji.unicode_eq("☑️") {
                if let Some(skin_upload) = ctx
                    .clone()
                    .data
                    .write()
                    .await
                    .get_mut::<SkinUploads>()
                    .unwrap()
                    .uploads
                    .get_mut(&add_reaction.user_id.unwrap())
                {
                    skin_upload
                        .reaction_list
                        .insert(add_reaction.message_id, add_reaction.user_id.unwrap());
                    skin_upload
                        .skins_try_upload
                        .insert(add_reaction.message_id, SkinToUploadDB::Community);
                    skin_upload.notify.notify_one();
                }
            }
        }
    }

    async fn ready(&self, ctx: Context, _ready: Ready) {
        let guild_id = GuildId::new(
            env::var("GUILD_ID")
                .expect("Expected GUILD_ID in environment")
                .parse()
                .expect("GUILD_ID must be an integer"),
        );

        let upload_cmd = CreateCommand::new("upload")
            .description("Upload a skin to the database")
            .dm_permission(false);
        let upload_finish_cmd = CreateCommand::new("upload_finish")
            .description("Finish an upload, previously started with the `/upload` command")
            .dm_permission(false);

        let upload_cancel_cmd = CreateCommand::new("upload_cancel")
            .description("Cancel an ongoing upload, that was started using the `/upload` command")
            .dm_permission(false);

        if let Err(_) = guild_id
            .set_commands(
                &ctx.http,
                vec![upload_cmd, upload_finish_cmd, upload_cancel_cmd],
            )
            .await
        {
            // ignore for now
        }
    }
}

pub enum SkinUploadState {
    Collecting,
    Uploading,
    Cancelled,
}

#[derive(Clone, PartialEq)]
pub enum SkinToUploadDB {
    Normal,
    Community,
}

impl SkinToUploadDB {
    pub fn to_string(&self) -> String {
        match self {
            SkinToUploadDB::Normal => "normal".to_string(),
            SkinToUploadDB::Community => "community".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct SkinToUpload {
    author: String,
    license: String,
    file_256x128: Vec<u8>,
    file_512x256: Vec<u8>,
    database: SkinToUploadDB,
    original_msg_id: MessageId,
}

pub struct SkinUploadItem {
    notify: Arc<Notify>,
    reaction_list: LinkedHashMap<MessageId, UserId>,
    skins_try_upload: LinkedHashMap<MessageId, SkinToUploadDB>,
    errors: VecDeque<String>,
    state: SkinUploadState,
    skins_to_upload: LinkedHashMap<String, SkinToUpload>,
}

pub struct SkinUploads {
    uploads: HashMap<UserId, SkinUploadItem>,
    upload_lock: Arc<Mutex<()>>,
}

impl TypeMapKey for SkinUploads {
    type Value = SkinUploads;
}

#[tokio::main]
async fn main() {
    let framework = StandardFramework::new();

    /*
    for ez debugging
    env::set_var("GUILD_ID", "");
    env::set_var("ROLE_ID", "");
    env::set_var(
        "DISCORD_TOKEN",
        "",
    );
    env::set_var("USERNAME", "");
    env::set_var("PASSWORD", "");
    */

    // Login with a bot token from the environment
    let token = env::var("DISCORD_TOKEN").expect("token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .framework(framework)
        .await
        .expect("Error creating client");

    let skin_uploads = SkinUploads {
        uploads: Default::default(),
        upload_lock: Default::default(),
    };
    client
        .data
        .write()
        .await
        .insert::<SkinUploads>(skin_uploads);

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }
}
