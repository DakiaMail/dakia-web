# Public HTML email test fixtures

`mailchimp-body-image.html` and `mailchimp-two-column.html` are unmodified MailChimp Email Blueprints:

- https://github.com/mailchimp/email-blueprints/blob/master/responsive-templates/base_boxed_basic_body_image_query.html
- https://github.com/mailchimp/email-blueprints/blob/master/responsive-templates/base_boxed_2column_query.html

Copyright MailChimp. These two files are licensed under Creative Commons Attribution-ShareAlike 3.0 Unported, not the surrounding application's MPL-2.0 license.

License: https://creativecommons.org/licenses/by-sa/3.0/
Upstream license declaration: https://github.com/mailchimp/email-blueprints/blob/master/README.md

Downloaded 2026-10-31. Tests never contact their image/link hosts. These are public production-style email templates, not owner emails or evidence of live provider acceptance. The generated newsletter fixture separately covers a fixed-size loaded image and hostile markup.

The synthetic shipment in `email-layouts.spec.ts` covers `width="60px"` and
`height="60px"` image attributes, comparing decoded dimensions with the original
HTML at six widths in both themes. It also checks keyboard opening, image
consent, scrolling and reload. To test an owner-approved private copy, set
`DAKIA_PRIVATE_SHIPMENT` to a private directory containing `message.json` (`html`)
and `images.json` (source URL to raster data URL). Screenshots and geometry
reports stay in that directory. Provider APIs are mocked and external browser
requests are blocked. Keep private bodies, personalized URLs and screenshots out
of Git.
