document.addEventListener("DOMContentLoaded", () => {
  const isRussian = document.documentElement.lang.toLowerCase().startsWith("ru");
  const username = document.getElementById("username");
  const password = document.getElementById("password");

  if (username) {
    username.placeholder = isRussian ? "Введите логин или email" : "Enter your username or email";
  }

  if (password) {
    password.placeholder = isRussian ? "Введите пароль" : "Enter your password";
  }
});
