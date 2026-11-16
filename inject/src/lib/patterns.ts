// Field name patterns for form detection (spec 11 §3, rule 3): Mado's own
// tables, matched against a field's name, id, placeholder, aria-label and
// label text. Each covers English, Spanish, French, German, Portuguese,
// Italian, Dutch, Japanese, Chinese and Russian where the words differ.
// First match wins, so more specific classes come first.

import type { FieldClass } from "./formdetect";

export const PATTERNS: [FieldClass, RegExp][] = [
  // Login identifiers that accept several kinds: "Email or phone".
  [
    "username",
    /(e.?mail|phone|mobile|user).{0,24}(\bor\b|\/|\bo\b|\bou\b|\boder\b).{0,24}(e.?mail|phone|mobile|user|number)/i,
  ],
  [
    "otp",
    /one.?time|\botp\b|2fa|two.?factor|totp|\bmfa\b|verification.?code|verify.?code|auth(entication)?.?code|sms.?code|login.?code|código de verificación|code de vérification|bestätigungscode|verificatiecode|codice di verifica|確認コード|認証コード|验证码|код подтверждения/i,
  ],
  [
    "ccNumber",
    /card.?(number|num|no\b)|cc.?num|ccnum|cardnum|credit.?card|debit.?card|n[uú]mero de (la )?tarjeta|num[ée]ro de (la )?carte|kartennummer|numero (della )?carta|kaartnummer|número do cartão|カード番号|卡号|номер карты/i,
  ],
  [
    "ccName",
    /name.?on.?card|card.?holder|cardholder|cc.?name|holder.?name|nombre (del|en la) (titular|tarjeta)|titulaire|karteninhaber|intestatario|kaarthouder|nome no cartão|名義|持卡人/i,
  ],
  ["ccExp", /mm.?\/.?(yy|aa|jj)|month.?\/.?year/i],
  [
    "ccExpMonth",
    /(exp|expir|vencim|ablauf|scaden|verloop|validade)\w*.{0,8}(month|mm\b|mes|mois|monat|mese|maand)|cc.?month|card.?month|exp.?m\b/i,
  ],
  [
    "ccExpYear",
    /(exp|expir|vencim|ablauf|scaden|verloop|validade)\w*.{0,8}(year|yy|año|ann[ée]e|jahr|anno|jaar|ano)|cc.?year|card.?year|exp.?y\b/i,
  ],
  [
    "ccExp",
    /expir|exp.?date|expiry|\bexp\b|mm.?\/.?yy|valid.?thru|vencimiento|ablaufdatum|date d.expiration|scadenza|vervaldatum|validade|有効期限|有效期|срок действия/i,
  ],
  [
    "ccCsc",
    /\bcvc|\bcvv|\bcsc|\bcvn|card.?verification|card.?security|security.?code|c[óo]digo de seguridad|cryptogramme|prüfnummer|codice di sicurezza|beveiligingscode|セキュリティコード|安全码/i,
  ],
  ["email", /e.?mail|courriel|correo|メール|邮箱|电子邮件|почта/i],
  [
    "username",
    /user.?name|\blogin\b|user.?id|account.?(name|id)|\bbenutzer|usuario|identifiant|utilisateur|nome utente|gebruikersnaam|ユーザー|用户名|логин|^user$/i,
  ],
  [
    "givenName",
    /first.?name|given.?name|\bfname|forename|vorname|pr[ée]nom|\bnombre\b(?!.*(apellido|complet))|primeiro nome|voornaam|^名$|名字|имя/i,
  ],
  [
    "familyName",
    /last.?name|surname|family.?name|\blname|nachname|apellido|nom de famille|^nom$|cognome|achternaam|sobrenome|^姓$|姓氏|фамилия/i,
  ],
  [
    "name",
    /full.?name|^name$|your.?name|\bname\b|nom complet|nombre completo|nome completo|vollständiger name|volledige naam|氏名|お名前|姓名|фио/i,
  ],
  [
    "organization",
    /company|organi[sz]ation|business.?name|\bfirma|empresa|entreprise|soci[ée]t[ée]|azienda|bedrijf|会社|公司/i,
  ],
  [
    "addressLine2",
    /address.?(line)?.?2|addr.?2|\bapt\b|apartment|suite|\bunit\b|adresszusatz|compl[ée]ment|piso|appartamento|toevoeging|建物名|门牌/i,
  ],
  ["addressLine3", /address.?(line)?.?3|addr.?3/i],
  [
    "addressLine1",
    /address.?(line)?.?1|addr.?1|street|address|anschrift|stra(ß|ss)e|direcci[óo]n|adresse|\brue\b|indirizzo|endereço|straat|住所|番地|地址|адрес/i,
  ],
  [
    "city",
    /city|town|locality|suburb|\bstadt|\bort\b|ciudad|localidad|ville|citt[àa]|cidade|plaats|woonplaats|市区町村|城市|город/i,
  ],
  [
    "region",
    /state|province|region|county|bundesland|provincia|estado|d[ée]partement|regione|provincie|都道府県|省份|^省$|область/i,
  ],
  [
    "postalCode",
    /\bzip|postal|post.?code|postcode|\bplz\b|c[óo]digo postal|code postal|\bcap\b|cep\b|郵便番号|邮编|邮政编码|индекс/i,
  ],
  ["country", /country|nation|\bland\b|pa[íi]s|\bpays\b|paese|国名|国家|страна/i],
  [
    "tel",
    /phone|\btel\b|telephone|mobile|\bcell|telefon|tel[ée]fono|t[ée]l[ée]phone|telefono|telefoon|電話|电话|手机|телефон/i,
  ],
  [
    "bday",
    /birth|\bbday|\bdob\b|geburts|nacimiento|naissance|nascita|geboorte|生年月日|出生|рождения/i,
  ],
];

/** Words for "new" or "again" next to a password field. */
export const NEW_PASSWORD =
  /new|confirm|repeat|re.?type|again|verify|\bneu|nouveau|nueva|nuevo|wiederhol|best[äa]tig|confirmer|confirmar|conferma|nieuw|bevestig|新しい|確認|确认|新密码|повтор|новый/i;

/** A form's own text that marks it as creating an account. */
export const SIGN_UP =
  /sign.?up|register|registration|create.{0,12}account|join now|get started|registr|inscri|crear cuenta|cr[ée]er (un )?compte|konto erstellen|crea (un )?account|account aanmaken|criar conta|新規登録|注册|регистрац/i;

/** Text around a short code field that marks a one-time code. */
export const OTP_WORDS = /code|c[óo]digo|pin\b|token|コード|验证|код/i;
