#include <curl/curl.h>
#include <json-c/json.h>

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <conio.h>
#include <sys/stat.h>
#include <windows.h>
#else
#include <sys/stat.h>
#include <termios.h>
#include <unistd.h>
#endif

#define DEFAULT_BASE_URL "https://ai.openjvspace.com"
#define DEFAULT_USERNAME "test"
#define MAX_RESPONSE_BYTES (8U * 1024U * 1024U)
#define MAX_ATTACHMENTS 10U
#define MAX_PASSWORD_BYTES 1024U
#define ERROR_BYTES 768U

struct api_error {
  char message[ERROR_BYTES];
};

struct response_buffer {
  char *data;
  size_t size;
  bool exceeded;
};

struct http_response {
  long status;
  struct response_buffer body;
};

struct api_client {
  char *base_url;
  char *access_token;
  long request_timeout_seconds;
};

struct options {
  const char *question;
  const char *base_url;
  const char *username;
  const char *files[MAX_ATTACHMENTS];
  size_t file_count;
  const char *conversation_id;
  double poll_interval_seconds;
  double wait_timeout_seconds;
  bool print_json;
  bool help;
};

static int set_error(struct api_error *error, const char *format, ...) {
  va_list arguments;
  va_start(arguments, format);
  (void)vsnprintf(error->message, sizeof(error->message), format, arguments);
  va_end(arguments);
  return -1;
}

static char *duplicate_string(const char *value) {
  const size_t length = strlen(value);
  char *copy = malloc(length + 1U);
  if (copy != NULL) {
    memcpy(copy, value, length + 1U);
  }
  return copy;
}

static void clear_secret(char *value) {
  if (value == NULL) {
    return;
  }
  volatile unsigned char *cursor = (volatile unsigned char *)value;
  const size_t length = strlen(value);
  for (size_t index = 0; index < length; ++index) {
    cursor[index] = 0;
  }
}

static char *url_part(CURLU *url, CURLUPart part) {
  char *value = NULL;
  if (curl_url_get(url, part, &value, 0) != CURLUE_OK) {
    return NULL;
  }
  return value;
}

static void lowercase_ascii(char *value) {
  if (value == NULL) {
    return;
  }
  for (; *value != '\0'; ++value) {
    *value = (char)tolower((unsigned char)*value);
  }
}

static int validate_base_url(const char *input, char **output,
                             struct api_error *error) {
  if (input == NULL || *input == '\0') {
    return set_error(error, "The API base URL is required.");
  }
  char *candidate = duplicate_string(input);
  if (candidate == NULL) {
    return set_error(error, "Out of memory.");
  }
  size_t length = strlen(candidate);
  while (length > 0U && candidate[length - 1U] == '/') {
    candidate[--length] = '\0';
  }

  CURLU *url = curl_url();
  if (url == NULL ||
      curl_url_set(url, CURLUPART_URL, candidate, 0) != CURLUE_OK) {
    if (url != NULL) {
      curl_url_cleanup(url);
    }
    free(candidate);
    return set_error(error, "The API base URL is invalid.");
  }

  char *scheme = url_part(url, CURLUPART_SCHEME);
  char *host = url_part(url, CURLUPART_HOST);
  char *path = url_part(url, CURLUPART_PATH);
  char *user = url_part(url, CURLUPART_USER);
  char *password = url_part(url, CURLUPART_PASSWORD);
  char *query = url_part(url, CURLUPART_QUERY);
  char *fragment = url_part(url, CURLUPART_FRAGMENT);
  lowercase_ascii(scheme);
  lowercase_ascii(host);

  const bool loopback_http =
      scheme != NULL && host != NULL && strcmp(scheme, "http") == 0 &&
      (strcmp(host, "127.0.0.1") == 0 || strcmp(host, "localhost") == 0 ||
       strcmp(host, "::1") == 0);
  const bool valid = scheme != NULL && host != NULL && *host != '\0' &&
                     (strcmp(scheme, "https") == 0 || loopback_http) &&
                     user == NULL && password == NULL && query == NULL &&
                     fragment == NULL &&
                     (path == NULL || *path == '\0' || strcmp(path, "/") == 0);

  curl_free(scheme);
  curl_free(host);
  curl_free(path);
  curl_free(user);
  curl_free(password);
  curl_free(query);
  curl_free(fragment);
  curl_url_cleanup(url);

  if (!valid) {
    free(candidate);
    return set_error(
        error,
        "The API base URL must be an HTTPS origin, or loopback HTTP for local "
        "development.");
  }
  *output = candidate;
  return 0;
}

static void response_buffer_free(struct response_buffer *buffer) {
  free(buffer->data);
  buffer->data = NULL;
  buffer->size = 0U;
  buffer->exceeded = false;
}

static void http_response_free(struct http_response *response) {
  response_buffer_free(&response->body);
  response->status = 0;
}

static size_t append_response(char *data, size_t size, size_t count,
                              void *userdata) {
  struct response_buffer *buffer = userdata;
  if (size != 0U && count > SIZE_MAX / size) {
    buffer->exceeded = true;
    return 0U;
  }
  const size_t bytes = size * count;
  if (bytes > MAX_RESPONSE_BYTES || buffer->size > MAX_RESPONSE_BYTES - bytes) {
    buffer->exceeded = true;
    return 0U;
  }
  char *expanded = realloc(buffer->data, buffer->size + bytes + 1U);
  if (expanded == NULL) {
    return 0U;
  }
  buffer->data = expanded;
  memcpy(buffer->data + buffer->size, data, bytes);
  buffer->size += bytes;
  buffer->data[buffer->size] = '\0';
  return bytes;
}

static int append_header(struct curl_slist **headers, const char *value,
                         struct api_error *error) {
  struct curl_slist *expanded = curl_slist_append(*headers, value);
  if (expanded == NULL) {
    return set_error(error, "Could not prepare HTTP headers.");
  }
  *headers = expanded;
  return 0;
}

static int build_headers(const struct api_client *client, bool authenticated,
                         bool json_body, bool disable_expect,
                         struct curl_slist **headers, struct api_error *error) {
  if (append_header(headers, "Accept: application/json", error) != 0 ||
      append_header(headers, "User-Agent: JV-AI-C-Example/1.0", error) != 0 ||
      append_header(headers, "X-JV-CSRF: 1", error) != 0) {
    return -1;
  }
  if (json_body &&
      append_header(headers, "Content-Type: application/json", error) != 0) {
    return -1;
  }
  if (disable_expect && append_header(headers, "Expect:", error) != 0) {
    return -1;
  }
  if (authenticated) {
    if (client->access_token == NULL || *client->access_token == '\0') {
      return set_error(error, "Authentication is required.");
    }
    const size_t length = strlen(client->access_token) + 23U;
    char *authorization = malloc(length);
    if (authorization == NULL) {
      return set_error(error, "Out of memory.");
    }
    (void)snprintf(authorization, length, "Authorization: Bearer %s",
                   client->access_token);
    const int result = append_header(headers, authorization, error);
    clear_secret(authorization);
    free(authorization);
    if (result != 0) {
      return -1;
    }
  }
  return 0;
}

static int configure_curl(CURL *curl, const struct api_client *client,
                          const char *path, struct http_response *response,
                          struct api_error *error) {
  const size_t url_length = strlen(client->base_url) + strlen(path) + 1U;
  char *url = malloc(url_length);
  if (url == NULL) {
    return set_error(error, "Out of memory.");
  }
  (void)snprintf(url, url_length, "%s%s", client->base_url, path);

#define SETOPT(option, value, message)                                         \
  do {                                                                         \
    if (curl_easy_setopt(curl, (option), (value)) != CURLE_OK) {               \
      free(url);                                                               \
      return set_error(error, "%s", (message));                                \
    }                                                                          \
  } while (0)

  SETOPT(CURLOPT_URL, url, "Could not prepare the request URL.");
#if LIBCURL_VERSION_NUM >= 0x075500
  SETOPT(CURLOPT_PROTOCOLS_STR, "http,https",
         "Could not restrict request protocols.");
#else
  SETOPT(CURLOPT_PROTOCOLS, CURLPROTO_HTTP | CURLPROTO_HTTPS,
         "Could not restrict request protocols.");
#endif
  SETOPT(CURLOPT_FOLLOWLOCATION, 0L, "Could not disable redirects.");
  SETOPT(CURLOPT_SSL_VERIFYPEER, 1L,
         "Could not enable TLS certificate verification.");
  SETOPT(CURLOPT_SSL_VERIFYHOST, 2L,
         "Could not enable TLS hostname verification.");
  SETOPT(CURLOPT_CONNECTTIMEOUT, 20L,
         "Could not configure the connection timeout.");
  SETOPT(CURLOPT_TIMEOUT, client->request_timeout_seconds,
         "Could not configure the request timeout.");
  SETOPT(CURLOPT_NOSIGNAL, 1L, "Could not configure the HTTP request.");
  SETOPT(CURLOPT_WRITEFUNCTION, append_response,
         "Could not configure response handling.");
  SETOPT(CURLOPT_WRITEDATA, &response->body,
         "Could not configure response handling.");

#undef SETOPT
  free(url);
  return 0;
}

static int perform_request(CURL *curl, struct http_response *response,
                           const char *transport_message,
                           struct api_error *error) {
  const CURLcode result = curl_easy_perform(curl);
  if (result != CURLE_OK) {
    if (response->body.exceeded) {
      return set_error(error,
                       "The JV AI API response exceeded the safe limit.");
    }
    return set_error(error, "%s", transport_message);
  }
  if (curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &response->status) !=
      CURLE_OK) {
    return set_error(error, "Could not read the HTTP status.");
  }
  return 0;
}

static int http_request(const struct api_client *client, const char *method,
                        const char *path, const char *body, bool json_body,
                        bool authenticated, struct http_response *response,
                        struct api_error *error) {
  memset(response, 0, sizeof(*response));
  CURL *curl = curl_easy_init();
  struct curl_slist *headers = NULL;
  if (curl == NULL) {
    return set_error(error, "Could not create an HTTP request.");
  }
  int status = -1;
  if (configure_curl(curl, client, path, response, error) != 0 ||
      build_headers(client, authenticated, json_body, false, &headers, error) !=
          0 ||
      curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers) != CURLE_OK) {
    goto cleanup;
  }
  if (strcmp(method, "POST") == 0) {
    const char *request_body = body == NULL ? "" : body;
    if (curl_easy_setopt(curl, CURLOPT_POST, 1L) != CURLE_OK ||
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, request_body) != CURLE_OK ||
        curl_easy_setopt(curl, CURLOPT_POSTFIELDSIZE_LARGE,
                         (curl_off_t)strlen(request_body)) != CURLE_OK) {
      (void)set_error(error, "Could not prepare the POST request.");
      goto cleanup;
    }
  } else if (strcmp(method, "GET") != 0) {
    (void)set_error(error, "Unsupported HTTP method.");
    goto cleanup;
  }
  status =
      perform_request(curl, response, "Could not reach the JV AI API.", error);

cleanup:
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);
  if (status != 0) {
    http_response_free(response);
  }
  return status;
}

static json_object *parse_json_body(const struct http_response *response,
                                    struct api_error *error) {
  if (response->body.data == NULL) {
    (void)set_error(error, "The JV AI API returned an empty response.");
    return NULL;
  }
  struct json_tokener *tokener = json_tokener_new();
  if (tokener == NULL) {
    (void)set_error(error, "Out of memory.");
    return NULL;
  }
  json_object *payload = json_tokener_parse_ex(tokener, response->body.data,
                                               (int)response->body.size);
  const enum json_tokener_error parse_error = json_tokener_get_error(tokener);
  json_tokener_free(tokener);
  if (parse_error != json_tokener_success || payload == NULL ||
      !json_object_is_type(payload, json_type_object)) {
    if (payload != NULL) {
      json_object_put(payload);
    }
    (void)set_error(error, "The JV AI API returned invalid JSON.");
    return NULL;
  }
  return payload;
}

static const char *json_string_field(json_object *object, const char *key) {
  json_object *value = NULL;
  if (object == NULL || !json_object_object_get_ex(object, key, &value) ||
      !json_object_is_type(value, json_type_string)) {
    return NULL;
  }
  return json_object_get_string(value);
}

static int add_json_value(json_object *object, const char *key,
                          json_object *value, struct api_error *error) {
  if (value == NULL) {
    return set_error(error, "Could not prepare JSON request data.");
  }
  if (json_object_object_add(object, key, value) != 0) {
    json_object_put(value);
    return set_error(error, "Could not prepare JSON request data.");
  }
  return 0;
}

static int safe_http_error(const struct http_response *response,
                           struct api_error *error) {
  const char *code = "JV-HTTP";
  const char *message = NULL;
  json_object *payload = parse_json_body(response, error);
  if (payload != NULL) {
    json_object *api_error = NULL;
    if (json_object_object_get_ex(payload, "error", &api_error) &&
        json_object_is_type(api_error, json_type_object)) {
      const char *candidate_code = json_string_field(api_error, "code");
      const char *candidate_message = json_string_field(api_error, "message");
      if (candidate_code != NULL) {
        code = candidate_code;
      }
      if (candidate_message != NULL) {
        message = candidate_message;
      }
    }
  }
  int result;
  if (message != NULL) {
    result = set_error(error, "%s: %s", code, message);
  } else {
    result = set_error(error, "%s: The JV AI API returned HTTP %ld.", code,
                       response->status);
  }
  if (payload != NULL) {
    json_object_put(payload);
  }
  return result;
}

static json_object *require_json_status(const struct http_response *response,
                                        long expected,
                                        struct api_error *error) {
  if (response->status != expected) {
    (void)safe_http_error(response, error);
    return NULL;
  }
  return parse_json_body(response, error);
}

static int client_initialize(struct api_client *client, const char *base_url,
                             struct api_error *error) {
  memset(client, 0, sizeof(*client));
  client->request_timeout_seconds = 120L;
  return validate_base_url(base_url, &client->base_url, error);
}

static void client_destroy(struct api_client *client) {
  clear_secret(client->access_token);
  free(client->access_token);
  free(client->base_url);
  memset(client, 0, sizeof(*client));
}

static int client_login(struct api_client *client, const char *username,
                        const char *password, struct api_error *error) {
  if (username == NULL || *username == '\0' || password == NULL ||
      *password == '\0') {
    return set_error(error, "Username and password are required.");
  }
  json_object *request = json_object_new_object();
  if (request == NULL) {
    return set_error(error, "Could not prepare the login request.");
  }
  if (add_json_value(request, "username", json_object_new_string(username),
                     error) != 0 ||
      add_json_value(request, "password", json_object_new_string(password),
                     error) != 0 ||
      add_json_value(request, "remember_me", json_object_new_boolean(false),
                     error) != 0) {
    json_object_put(request);
    return set_error(error, "Could not prepare the login request.");
  }
  const char *body =
      json_object_to_json_string_ext(request, JSON_C_TO_STRING_PLAIN);
  struct http_response response;
  if (http_request(client, "POST", "/v1/auth/login", body, true, false,
                   &response, error) != 0) {
    json_object_put(request);
    return -1;
  }
  json_object_put(request);
  json_object *payload = require_json_status(&response, 200L, error);
  http_response_free(&response);
  if (payload == NULL) {
    return -1;
  }
  const char *token = json_string_field(payload, "access_token");
  if (token == NULL || *token == '\0') {
    json_object_put(payload);
    return set_error(error,
                     "The login response did not include a bearer token.");
  }
  client->access_token = duplicate_string(token);
  if (client->access_token == NULL) {
    json_object_put(payload);
    return set_error(error, "Out of memory.");
  }
  json_object *user = NULL;
  const char *display_name = username;
  if (json_object_object_get_ex(payload, "user", &user) &&
      json_object_is_type(user, json_type_object)) {
    const char *candidate = json_string_field(user, "username");
    if (candidate != NULL) {
      display_name = candidate;
    }
  }
  fprintf(stderr, "Authenticated as %s.\n", display_name);
  json_object_put(payload);
  return 0;
}

static bool regular_file(const char *path) {
#ifdef _WIN32
  struct _stat status;
  return _stat(path, &status) == 0 && (status.st_mode & _S_IFREG) != 0;
#else
  struct stat status;
  return lstat(path, &status) == 0 && S_ISREG(status.st_mode) &&
         !S_ISLNK(status.st_mode);
#endif
}

static const char *base_name(const char *path) {
  const char *slash = strrchr(path, '/');
  const char *backslash = strrchr(path, '\\');
  const char *separator = slash;
  if (backslash != NULL && (separator == NULL || backslash > separator)) {
    separator = backslash;
  }
  return separator == NULL ? path : separator + 1;
}

static int add_mime_text(curl_mime *mime, const char *name, const char *value,
                         struct api_error *error) {
  curl_mimepart *part = curl_mime_addpart(mime);
  if (part == NULL || curl_mime_name(part, name) != CURLE_OK ||
      curl_mime_data(part, value, CURL_ZERO_TERMINATED) != CURLE_OK) {
    return set_error(error, "Could not prepare multipart request data.");
  }
  return 0;
}

static json_object *client_submit_job(struct api_client *client,
                                      const struct options *options,
                                      struct api_error *error) {
  if (options->question == NULL ||
      strspn(options->question, " \t\r\n") == strlen(options->question)) {
    (void)set_error(error, "Question text must not be empty.");
    return NULL;
  }
  if (options->conversation_id != NULL && *options->conversation_id == '\0') {
    (void)set_error(error, "conversation_id must not be empty.");
    return NULL;
  }
  for (size_t index = 0; index < options->file_count; ++index) {
    if (!regular_file(options->files[index])) {
      (void)set_error(error, "Attachment is not a regular file: %s",
                      options->files[index]);
      return NULL;
    }
  }

  struct http_response response;
  memset(&response, 0, sizeof(response));
  CURL *curl = curl_easy_init();
  struct curl_slist *headers = NULL;
  curl_mime *mime = NULL;
  if (curl == NULL) {
    (void)set_error(error, "Could not create an HTTP request.");
    return NULL;
  }
  json_object *payload = NULL;
  if (configure_curl(curl, client, "/v1/jobs", &response, error) != 0 ||
      build_headers(client, true, false, true, &headers, error) != 0 ||
      curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers) != CURLE_OK) {
    goto cleanup;
  }
  mime = curl_mime_init(curl);
  if (mime == NULL ||
      add_mime_text(mime, "text", options->question, error) != 0) {
    if (mime == NULL) {
      (void)set_error(error, "Could not prepare multipart request data.");
    }
    goto cleanup;
  }
  if (options->conversation_id != NULL &&
      add_mime_text(mime, "conversation_id", options->conversation_id, error) !=
          0) {
    goto cleanup;
  }
  for (size_t index = 0; index < options->file_count; ++index) {
    curl_mimepart *part = curl_mime_addpart(mime);
    if (part == NULL || curl_mime_name(part, "files") != CURLE_OK ||
        curl_mime_filedata(part, options->files[index]) != CURLE_OK ||
        curl_mime_filename(part, base_name(options->files[index])) !=
            CURLE_OK) {
      (void)set_error(error, "Could not prepare an attachment.");
      goto cleanup;
    }
  }
  if (curl_easy_setopt(curl, CURLOPT_MIMEPOST, mime) != CURLE_OK) {
    (void)set_error(error, "Could not prepare the job submission.");
    goto cleanup;
  }
  if (perform_request(
          curl, &response,
          "Job submission did not return a definite result. Do not "
          "automatically repeat this POST because the first job may already "
          "exist.",
          error) != 0) {
    goto cleanup;
  }
  payload = require_json_status(&response, 202L, error);
  if (payload != NULL) {
    const char *job_id = json_string_field(payload, "id");
    if (job_id == NULL || *job_id == '\0') {
      json_object_put(payload);
      payload = NULL;
      (void)set_error(error, "The job response did not include a job ID.");
    }
  }

cleanup:
  curl_mime_free(mime);
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);
  http_response_free(&response);
  return payload;
}

static json_object *client_get_job(struct api_client *client,
                                   const char *job_id,
                                   struct api_error *error) {
  const size_t path_length = strlen(job_id) + 11U;
  char *path = malloc(path_length);
  if (path == NULL) {
    (void)set_error(error, "Out of memory.");
    return NULL;
  }
  (void)snprintf(path, path_length, "/v1/jobs/%s", job_id);
  struct http_response response;
  const int request_status =
      http_request(client, "GET", path, NULL, false, true, &response, error);
  free(path);
  if (request_status != 0) {
    return NULL;
  }
  json_object *payload = require_json_status(&response, 200L, error);
  http_response_free(&response);
  return payload;
}

static double monotonic_seconds(void) {
#ifdef _WIN32
  return (double)GetTickCount64() / 1000.0;
#else
  struct timespec value;
  if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) {
    return 0.0;
  }
  return (double)value.tv_sec + (double)value.tv_nsec / 1000000000.0;
#endif
}

static void sleep_seconds(double seconds) {
#ifdef _WIN32
  Sleep((DWORD)(seconds * 1000.0));
#else
  struct timespec requested;
  requested.tv_sec = (time_t)seconds;
  requested.tv_nsec =
      (long)((seconds - (double)requested.tv_sec) * 1000000000.0);
  while (nanosleep(&requested, &requested) != 0 && errno == EINTR) {
  }
#endif
}

static json_object *client_wait_for_job(struct api_client *client,
                                        const char *job_id,
                                        double poll_interval_seconds,
                                        double wait_timeout_seconds,
                                        struct api_error *error) {
  const double deadline = monotonic_seconds() + wait_timeout_seconds;
  char last_status[64] = "";
  char last_phase[96] = "";
  while (true) {
    json_object *job = client_get_job(client, job_id, error);
    if (job == NULL) {
      return NULL;
    }
    const char *status = json_string_field(job, "status");
    const char *phase = json_string_field(job, "phase");
    status = status == NULL ? "unknown" : status;
    phase = phase == NULL ? "unknown" : phase;
    if (strcmp(status, last_status) != 0 || strcmp(phase, last_phase) != 0) {
      fprintf(stderr, "Status: %s (%s)", status, phase);
      json_object *position = NULL;
      if (json_object_object_get_ex(job, "queue_position", &position) &&
          json_object_is_type(position, json_type_int)) {
        fprintf(stderr, ", queue position %lld",
                (long long)json_object_get_int64(position));
      }
      fputc('\n', stderr);
      (void)snprintf(last_status, sizeof(last_status), "%s", status);
      (void)snprintf(last_phase, sizeof(last_phase), "%s", phase);
    }
    if (strcmp(status, "succeeded") == 0 || strcmp(status, "failed") == 0) {
      return job;
    }
    json_object_put(job);
    const double remaining = deadline - monotonic_seconds();
    if (remaining <= 0.0) {
      (void)set_error(
          error,
          "Local polling timed out. The server-side job was not cancelled; it "
          "can be polled again using the same job ID.");
      return NULL;
    }
    sleep_seconds(poll_interval_seconds < remaining ? poll_interval_seconds
                                                    : remaining);
  }
}

static int client_logout(struct api_client *client, struct api_error *error) {
  if (client->access_token == NULL) {
    return 0;
  }
  struct http_response response;
  const int request_status = http_request(client, "POST", "/v1/auth/logout", "",
                                          false, true, &response, error);
  clear_secret(client->access_token);
  free(client->access_token);
  client->access_token = NULL;
  if (request_status != 0) {
    return -1;
  }
  if (response.status != 204L) {
    (void)safe_http_error(&response, error);
    http_response_free(&response);
    return -1;
  }
  http_response_free(&response);
  return 0;
}

static void show_usage(const char *program) {
  printf("Usage: %s QUESTION [options]\n\n", program);
  puts("--file PATH                 Attach a file; repeat as needed");
  puts("--conversation-id ID        Continue an owned conversation");
  puts("--base-url URL              Override the API origin");
  puts("--username USERNAME         Override the default username");
  puts("--poll-interval SECONDS     Poll interval; default: 2");
  puts("--wait-timeout SECONDS      Local wait timeout; default: 3600");
  puts("--json                      Print complete public job JSON");
  puts("--help                      Show this help");
}

static int positive_number(const char *value, const char *option,
                           double *output, struct api_error *error) {
  char *end = NULL;
  errno = 0;
  const double parsed = strtod(value, &end);
  if (errno != 0 || end == value || *end != '\0' || !isfinite(parsed) ||
      parsed <= 0.0) {
    return set_error(error, "%s must be a positive number.", option);
  }
  *output = parsed;
  return 0;
}

static int parse_options(int argc, char **argv, struct options *options,
                         struct api_error *error) {
  memset(options, 0, sizeof(*options));
  options->base_url = getenv("JV_API_BASE_URL");
  options->username = getenv("JV_API_USERNAME");
  options->base_url =
      options->base_url == NULL ? DEFAULT_BASE_URL : options->base_url;
  options->username =
      options->username == NULL ? DEFAULT_USERNAME : options->username;
  options->poll_interval_seconds = 2.0;
  options->wait_timeout_seconds = 3600.0;
  if (argc < 2) {
    return set_error(error, "A question is required. Use --help for usage.");
  }
  for (int index = 1; index < argc; ++index) {
    if (strcmp(argv[index], "--help") == 0) {
      options->help = true;
      return 0;
    }
  }
  options->question = argv[1];
  for (int index = 2; index < argc; ++index) {
    const char *option = argv[index];
    if (strcmp(option, "--json") == 0) {
      options->print_json = true;
      continue;
    }
    if (index + 1 >= argc) {
      return set_error(error, "%s requires a value.", option);
    }
    const char *value = argv[++index];
    if (strcmp(option, "--file") == 0) {
      if (options->file_count >= MAX_ATTACHMENTS) {
        return set_error(error, "At most %u attachments are supported.",
                         MAX_ATTACHMENTS);
      }
      options->files[options->file_count++] = value;
    } else if (strcmp(option, "--conversation-id") == 0) {
      options->conversation_id = value;
    } else if (strcmp(option, "--base-url") == 0) {
      options->base_url = value;
    } else if (strcmp(option, "--username") == 0) {
      options->username = value;
    } else if (strcmp(option, "--poll-interval") == 0) {
      if (positive_number(value, option, &options->poll_interval_seconds,
                          error) != 0) {
        return -1;
      }
    } else if (strcmp(option, "--wait-timeout") == 0) {
      if (positive_number(value, option, &options->wait_timeout_seconds,
                          error) != 0) {
        return -1;
      }
    } else {
      return set_error(error, "Unknown option: %s", option);
    }
  }
  return 0;
}

static char *read_password(const char *username, struct api_error *error) {
  fprintf(stderr, "Password for %s: ", username);
  fflush(stderr);
  char buffer[MAX_PASSWORD_BYTES + 2U];
  memset(buffer, 0, sizeof(buffer));
#ifdef _WIN32
  size_t length = 0U;
  bool exceeded = false;
  while (true) {
    const int character = _getch();
    if (character == '\r' || character == '\n') {
      break;
    }
    if (character == '\b') {
      if (length > 0U) {
        --length;
      }
    } else if (character >= 0 && character <= 255 &&
               length < MAX_PASSWORD_BYTES) {
      buffer[length++] = (char)character;
    } else if (character >= 0 && character <= 255) {
      exceeded = true;
    }
  }
  fputc('\n', stderr);
  if (exceeded) {
    clear_secret(buffer);
    (void)set_error(error, "Password exceeds the safe input limit.");
    return NULL;
  }
#else
  struct termios previous;
  const bool terminal =
      isatty(STDIN_FILENO) && tcgetattr(STDIN_FILENO, &previous) == 0;
  if (terminal) {
    struct termios hidden = previous;
    hidden.c_lflag &= (tcflag_t)~ECHO;
    if (tcsetattr(STDIN_FILENO, TCSAFLUSH, &hidden) != 0) {
      (void)set_error(error, "Could not disable password echo.");
      return NULL;
    }
  }
  char *result = fgets(buffer, (int)sizeof(buffer), stdin);
  if (terminal) {
    (void)tcsetattr(STDIN_FILENO, TCSAFLUSH, &previous);
  }
  fputc('\n', stderr);
  if (result == NULL) {
    (void)set_error(error, "Could not read the password.");
    return NULL;
  }
  const size_t input_length = strlen(buffer);
  if (input_length > MAX_PASSWORD_BYTES) {
    clear_secret(buffer);
    (void)set_error(error, "Password exceeds the safe input limit.");
    return NULL;
  }
#endif
  buffer[strcspn(buffer, "\r\n")] = '\0';
  char *password = duplicate_string(buffer);
  clear_secret(buffer);
  if (password == NULL) {
    (void)set_error(error, "Out of memory.");
  }
  return password;
}

int main(int argc, char **argv) {
  struct api_error error = {0};
  struct options options;
  if (parse_options(argc, argv, &options, &error) != 0) {
    fprintf(stderr, "Error: %s\n", error.message);
    return 1;
  }
  if (options.help) {
    show_usage(argv[0]);
    return 0;
  }
  if (curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) {
    fputs("Error: Could not initialize libcurl.\n", stderr);
    return 1;
  }

  int exit_status = 1;
  struct api_client client;
  memset(&client, 0, sizeof(client));
  char *password = NULL;
  json_object *created = NULL;
  json_object *terminal = NULL;
  if (client_initialize(&client, options.base_url, &error) != 0) {
    goto cleanup;
  }
  const char *configured_password = getenv("JV_API_PASSWORD");
  password = configured_password == NULL
                 ? read_password(options.username, &error)
                 : duplicate_string(configured_password);
  if (password == NULL) {
    if (*error.message == '\0') {
      (void)set_error(&error, "Out of memory.");
    }
    goto cleanup;
  }
  if (client_login(&client, options.username, password, &error) != 0) {
    goto cleanup;
  }
  clear_secret(password);
  free(password);
  password = NULL;

  created = client_submit_job(&client, &options, &error);
  if (created == NULL) {
    goto cleanup;
  }
  const char *job_id = json_string_field(created, "id");
  const char *conversation_id = json_string_field(created, "conversation_id");
  fprintf(stderr, "Created job %s in conversation %s.\n", job_id,
          conversation_id == NULL ? "unknown" : conversation_id);

  terminal = client_wait_for_job(&client, job_id, options.poll_interval_seconds,
                                 options.wait_timeout_seconds, &error);
  if (terminal == NULL) {
    goto cleanup;
  }
  const char *status = json_string_field(terminal, "status");
  if (options.print_json) {
    puts(json_object_to_json_string_ext(terminal, JSON_C_TO_STRING_PRETTY));
  } else if (status != NULL && strcmp(status, "succeeded") == 0) {
    const char *answer = json_string_field(terminal, "answer");
    puts(answer == NULL ? "" : answer);
  } else {
    const char *code = json_string_field(terminal, "error_code");
    const char *message = json_string_field(terminal, "error_message");
    (void)set_error(&error, "%s: %s", code == NULL ? "JV-JOB" : code,
                    message == NULL ? "The JV AI job failed." : message);
    goto cleanup;
  }
  exit_status = 0;

cleanup:
  clear_secret(password);
  free(password);
  if (terminal != NULL) {
    json_object_put(terminal);
  }
  if (created != NULL) {
    json_object_put(created);
  }
  if (client.access_token != NULL) {
    struct api_error logout_error = {0};
    if (client_logout(&client, &logout_error) != 0) {
      fprintf(stderr, "Warning: %s\n", logout_error.message);
    }
  }
  client_destroy(&client);
  curl_global_cleanup();
  if (exit_status != 0) {
    fprintf(stderr, "Error: %s\n",
            *error.message == '\0' ? "Unexpected failure." : error.message);
  }
  return exit_status;
}
