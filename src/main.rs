mod agent;
mod icon_theme;
use futures::SinkExt;
use iced::Color;
use iced::Element;
use iced::Task;
use iced::widget::operation::focus;
use iced::window::Id;
use iced::{
    Alignment, Length,
    widget::{Space, button, checkbox, column, combo_box, container, row, svg, text, text_input},
};
use iced_exwlshell::reexport::KeyboardInteractivity;
use iced_exwlshell::reexport::OutputOption;
use iced_exwlshell::settings::LayerShellSettings;
use iced_exwlshell::settings::LayerSize;
use iced_exwlshell::settings::StartMode;
use iced_exwlshell::to_layer_message;
use iced_exwlshell::{
    daemon,
    reexport::NewLayerShellSettings,
    reexport::{Anchor, Layer},
};

use std::sync::LazyLock;

use futures::channel::mpsc::{self, Sender};

use crate::agent::init_agent;
use crate::icon_theme::IconTheme;

static INPUT_ID: LazyLock<iced::widget::Id> = LazyLock::new(iced::widget::Id::unique);

#[derive(Debug, Clone)]
pub enum PasswordMessage {
    SwitchUser(usize),
    Password(String),
    Canceled,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    PolkitComing {
        user_names: Vec<String>,
        action_id: String,
        icon_name: String,
    },
    PolkitPasswordRequest {
        prompt: String,
        message: String,
    },
    PolkitComplete,
    PolkitError(String),
    PolkitInfo(String),
    SwitchUser(UserInfo),
    SwitchInfo(bool),
    Cancel,
    Confirm,
    Opened(Id),
    IcedEvent(iced::Event),
    PolkitPwSender(Sender<PasswordMessage>),
    PasswordChange(String),
}

#[derive(Debug)]
struct PolkitDialog {
    error_message: String,
    info_message: String,
    message: String,
    prompt: String,
    password: String,
    current_id: Option<Id>,
    pw_sender: Option<Sender<PasswordMessage>>,
    info_icon: svg::Handle,
    user_names: combo_box::State<UserInfo>,
    current_user: Option<UserInfo>,
    user_count: usize,
    action_id: String,
    more_information: bool,
}

#[derive(Debug, Clone)]
struct UserInfo {
    name: String,
    index: usize,
}

impl std::fmt::Display for UserInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

const DIALOG_NAMESPACE: &str = "osd";

impl PolkitDialog {
    fn new() -> Self {
        Self {
            error_message: "".to_owned(),
            info_message: "".to_owned(),
            message: "".to_owned(),
            prompt: "Your password".to_owned(),
            password: "".to_owned(),
            current_id: None,
            pw_sender: None,
            info_icon: svg::Handle::default_icon(),
            user_names: combo_box::State::new(vec![]),
            current_user: None,
            user_count: 0,
            action_id: "".to_owned(),
            more_information: false,
        }
    }
}

fn update(dialog: &mut PolkitDialog, message: Message) -> Task<Message> {
    use iced::window::Action as WindowAction;
    use iced_runtime::Action;
    match message {
        Message::PolkitComing {
            user_names,
            action_id,
            icon_name,
        } => {
            let users: Vec<UserInfo> = user_names
                .into_iter()
                .enumerate()
                .map(|(index, name)| UserInfo { index, name })
                .collect();
            dialog.current_user = Some(users[0].clone());
            dialog.user_names = combo_box::State::new(users);
            dialog.info_icon = svg::Handle::from_icon(&icon_name);
            dialog.action_id = action_id;
            Task::none()
        }
        Message::SwitchUser(UserInfo { index, .. }) => {
            if let Some(pw_sender) = &mut dialog.pw_sender {
                let _ = pw_sender.try_send(PasswordMessage::SwitchUser(index));
            }
            Task::none()
        }
        Message::PolkitPasswordRequest { prompt, message } => {
            if dialog.current_id.is_some() {
                return Task::none();
            }
            dialog.prompt = prompt;
            dialog.message = message;
            Task::done(Message::NewLayerShell {
                settings: NewLayerShellSettings {
                    size: LayerSize::FILL,
                    exclusive_zone: Some(-1),
                    anchor: Anchor::all(),
                    layer: Layer::Overlay,
                    keyboard_interactivity: KeyboardInteractivity::Exclusive,
                    output_option: OutputOption::LastOutput,
                    ..Default::default()
                },
                id: iced::window::Id::unique(),
            })
        }
        Message::Opened(id) => {
            dialog.current_id = Some(id);
            focus(INPUT_ID.clone())
        }
        Message::Confirm => {
            if let Some(pw_sender) = &mut dialog.pw_sender {
                let _ = pw_sender.try_send(PasswordMessage::Password(dialog.password.clone()));
            }

            Task::none()
        }
        Message::PolkitComplete => {
            let id = dialog.current_id.take().unwrap();
            dialog.password.clear();
            iced_runtime::task::effect(Action::Window(WindowAction::Close(id)))
        }
        Message::Cancel => {
            if let Some(pw_sender) = &mut dialog.pw_sender {
                let _ = pw_sender.try_send(PasswordMessage::Canceled);
            }
            if let Some(id) = dialog.current_id.take() {
                return iced_runtime::task::effect(Action::Window(WindowAction::Close(id)));
            }
            dialog.password.clear();
            Task::none()
        }
        Message::PolkitError(err) => {
            dialog.error_message = err;
            Task::none()
        }
        Message::PolkitInfo(info) => {
            dialog.info_message = info;
            Task::none()
        }

        Message::SwitchInfo(enable) => {
            dialog.more_information = enable;
            Task::none()
        }
        Message::PasswordChange(password) => {
            dialog.password = password;
            Task::none()
        }
        Message::PolkitPwSender(sender) => {
            dialog.pw_sender = Some(sender);
            Task::none()
        }
        Message::IcedEvent(iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key,
            ..
        })) => match key {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
                Task::done(Message::Confirm)
            }
            _ => focus(INPUT_ID.clone()),
        },
        _ => Task::none(),
    }
}

fn view(dialog: &PolkitDialog, _id: iced::window::Id) -> Element<'_, Message> {
    let user_element: Element<'_, Message> = if dialog.user_count == 0
        && let Some(user) = &dialog.current_user
    {
        text(&user.name).size(30).color(iced::color!(766699)).into()
    } else {
        combo_box(
            &dialog.user_names,
            "select user",
            dialog.current_user.as_ref(),
            Message::SwitchUser,
        )
        .size(28.)
        .into()
    };
    let info: Element<'_, Message> = if dialog.more_information {
        column![
            checkbox(dialog.more_information)
                .label("extra info")
                .on_toggle(Message::SwitchInfo),
            row![
                Space::new().width(24.),
                text(&dialog.action_id).color(Color::WHITE).size(15),
                Space::new().width(Length::Fill)
            ]
        ]
        .spacing(5)
        .width(Length::Fill)
        .height(50)
        .into()
    } else {
        container(
            checkbox(dialog.more_information)
                .label("extra info")
                .on_toggle(Message::SwitchInfo)
                .width(Length::Fill),
        )
        .height(50)
        .into()
    };
    container(
        column![
            row![
                svg(dialog.info_icon.clone()).width(30.).height(30.),
                container(user_element).align_y(Alignment::Center),
                Space::new().width(Length::Fill)
            ]
            .spacing(7)
            .width(Length::Fill),
            Space::new().height(8),
            container(text(&dialog.message).size(25.).color(Color::WHITE)).center_x(Length::Fill),
            Space::new().height(5),
            text_input(&dialog.prompt, &dialog.password)
                .padding(10)
                .secure(true)
                .id(INPUT_ID.clone())
                .on_input(Message::PasswordChange),
            text(&dialog.info_message),
            text(&dialog.error_message).color(iced::color!(0xff0000)),
            info,
            Space::new().height(10),
            row![
                button("Confirm").on_press(Message::Confirm),
                Space::new().width(30.),
                button("Cancel").on_press(Message::Cancel),
            ]
        ]
        .align_x(Alignment::Center)
        .width(Length::Fixed(700.)),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn style(_dialog: &PolkitDialog, theme: &iced::Theme) -> iced::theme::Style {
    use iced::theme::Style;
    Style {
        background_color: iced::Color::from_rgba(0.2, 0.2, 0.2, 0.8),
        text_color: theme.palette().text,
    }
}

fn subscription(_dialog: &PolkitDialog) -> iced::Subscription<Message> {
    iced::Subscription::batch(vec![
        iced::window::open_events().map(Message::Opened),
        iced::event::listen().map(Message::IcedEvent),
        polkit_subscription(),
    ])
}

fn polkit_subscription() -> iced::Subscription<Message> {
    iced::Subscription::run(|| {
        iced::stream::channel(100, |mut sender: Sender<Message>| async move {
            let (pw_sender, pw_receiver) = mpsc::channel(1000);

            sender
                .send(Message::PolkitPwSender(pw_sender))
                .await
                .unwrap();
            let _connection = init_agent(pw_receiver, sender).await.unwrap();
            std::future::pending::<()>().await
        })
    })
}

fn main() -> iced_exwlshell::Result {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::time::LocalTime;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy()
                .add_directive("usvg=off".parse().unwrap())
                .add_directive("wgpu_hal::vulkan=off".parse().unwrap()),
        )
        .with_timer(LocalTime::rfc_3339())
        .init();
    daemon(PolkitDialog::new, DIALOG_NAMESPACE, update, view)
        .layer_settings(LayerShellSettings {
            start_mode: StartMode::Background,
            exclusive_zone: -1,
            anchor: Anchor::all(),
            ..Default::default()
        })
        .subscription(subscription)
        .style(style)
        .run()?;
    Ok(())
}
