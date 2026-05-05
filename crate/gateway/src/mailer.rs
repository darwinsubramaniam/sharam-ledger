use std::sync::Arc;

use anyhow::{Context, Result};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use tracing::{info, warn};

use common::config::SmtpConfig;

#[derive(Clone)]
pub struct Mailer {
    inner: Arc<MailerInner>,
}

struct MailerInner {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    app_base_url: String,
}

impl Mailer {
    pub fn new(cfg: &SmtpConfig) -> Result<Self> {
        let from: Mailbox = format!("{} <{}>", cfg.from_name, cfg.from_email)
            .parse()
            .with_context(|| format!("invalid smtp.from_email: {}", cfg.from_email))?;

        let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());

        let transport = match cfg.encryption.as_str() {
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
                .with_context(|| format!("smtp tls relay {}", cfg.host))?
                .port(cfg.port)
                .credentials(creds)
                .build(),
            "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                .with_context(|| format!("smtp starttls relay {}", cfg.host))?
                .port(cfg.port)
                .credentials(creds)
                .build(),
            "plain" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
                .port(cfg.port)
                .credentials(creds)
                .build(),
            other => anyhow::bail!("unknown smtp.encryption: {other}"),
        };

        Ok(Self {
            inner: Arc::new(MailerInner {
                transport,
                from,
                app_base_url: cfg.app_base_url.trim_end_matches('/').to_string(),
            }),
        })
    }

    pub fn invite_link(&self, slug: &str) -> String {
        format!("{}/ventures/{}", self.inner.app_base_url, slug)
    }

    /// Send the invite email. Failures are logged and propagated; callers
    /// that don't want to block the HTTP response should `tokio::spawn` this.
    pub async fn send_invite(&self, invite: InviteEmail<'_>) -> Result<()> {
        let to: Mailbox = invite
            .to_email
            .parse()
            .with_context(|| format!("invalid invitee email: {}", invite.to_email))?;

        let link = self.invite_link(invite.tenant_slug);
        let role_label = pretty_role(invite.role);

        let subject = format!(
            "You've been invited to {} on Sharam",
            invite.tenant_display_name
        );
        let text = build_text_body(&invite, &role_label, &link);
        let html = build_html_body(&invite, &role_label, &link);

        let message = Message::builder()
            .from(self.inner.from.clone())
            .to(to)
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(text),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html),
                    ),
            )
            .context("building invite email")?;

        match self.inner.transport.send(message).await {
            Ok(_) => {
                info!(to = %invite.to_email, slug = %invite.tenant_slug, "invite email sent");
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, to = %invite.to_email, slug = %invite.tenant_slug, "invite email failed");
                Err(e.into())
            }
        }
    }
}

pub struct InviteEmail<'a> {
    pub to_email: &'a str,
    pub tenant_slug: &'a str,
    pub tenant_display_name: &'a str,
    pub role: &'a str,
    pub invited_by_name: Option<&'a str>,
    pub invited_by_email: &'a str,
}

fn pretty_role(role: &str) -> String {
    let mut chars = role.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn build_text_body(invite: &InviteEmail<'_>, role_label: &str, link: &str) -> String {
    let inviter = invite
        .invited_by_name
        .map(|n| format!("{n} ({})", invite.invited_by_email))
        .unwrap_or_else(|| invite.invited_by_email.to_string());

    format!(
        "Hi,\n\n\
         {inviter} invited you to join \"{venture}\" on Sharam as {role}.\n\n\
         Open the venture: {link}\n\n\
         If you don't have an account yet, sign in with this email \
         ({email}) using Google and the invite will be applied automatically.\n\n\
         — Sharam\n",
        venture = invite.tenant_display_name,
        role = role_label,
        link = link,
        email = invite.to_email,
    )
}

fn build_html_body(invite: &InviteEmail<'_>, role_label: &str, link: &str) -> String {
    let inviter = invite
        .invited_by_name
        .map(|n| {
            format!(
                "{} (<a href=\"mailto:{}\">{}</a>)",
                html_escape(n),
                invite.invited_by_email,
                invite.invited_by_email
            )
        })
        .unwrap_or_else(|| {
            format!(
                "<a href=\"mailto:{}\">{}</a>",
                invite.invited_by_email, invite.invited_by_email
            )
        });

    format!(
        "<!doctype html>\
        <html><body style=\"font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;color:#111;line-height:1.5\">\
        <p>Hi,</p>\
        <p>{inviter} invited you to join <strong>{venture}</strong> on Sharam as <strong>{role}</strong>.</p>\
        <p><a href=\"{link}\" style=\"display:inline-block;padding:10px 16px;background:#111;color:#fff;text-decoration:none;border-radius:6px\">Open {venture}</a></p>\
        <p style=\"font-size:13px;color:#555\">Or paste this link into your browser: <br/><a href=\"{link}\">{link}</a></p>\
        <p style=\"font-size:13px;color:#555\">If you don't have an account yet, sign in with this email \
        (<strong>{email}</strong>) using Google and the invite will be applied automatically.</p>\
        <p style=\"font-size:13px;color:#888\">— Sharam</p>\
        </body></html>",
        venture = html_escape(invite.tenant_display_name),
        role = html_escape(role_label),
        link = link,
        email = html_escape(invite.to_email),
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
