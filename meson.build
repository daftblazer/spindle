project('spindle', 'rust', 
          version: '0.2.0',
    meson_version: '>= 1.0.0',
  default_options: [ 'warning_level=2', 'werror=false', ],
)

i18n = import('i18n')
gnome = import('gnome')

base_id = 'io.github.daftblazer.Spindle'
profile = get_option('profile')
if profile == 'development'
  application_id = base_id + '.Devel'
  gschema_path = '/io/github/daftblazer/Spindle/Devel/'
else
  application_id = base_id
  gschema_path = '/io/github/daftblazer/Spindle/'
endif




subdir('data')
subdir('src')
subdir('po')

gnome.post_install(
     glib_compile_schemas: true,
    gtk_update_icon_cache: true,
  update_desktop_database: true,
  update_mime_database: true,
)
